//! Finance's copy of People approval facts, replacing Bilimbi Payroll.PeopleSource.
//! Wire contracts: People payroll/README.md and claims/README.md, events v1.
use crate::{store, PayrollError, Result, MODULE};
use bilibi_authz::SystemScope;
use bilibi_database::{Change, Error};
use bilibi_events::{Event, Events, On, Subscribe, Subscriber, SubscriberFuture, SUBSCRIBERS};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Transaction};

const RUN_APPROVED: &str = "people/payroll.run_approved";
const CLAIM_APPROVED: &str = "people/claims.request_approved";
const CLAIM_REIMBURSED: &str = "people/claims.request_reimbursed";
const SOURCE_EVENTS: [&str; 3] = [RUN_APPROVED, CLAIM_APPROVED, CLAIM_REIMBURSED];

pub(crate) fn available(events: &Events) -> bool {
    SOURCE_EVENTS
        .iter()
        .all(|name| events.event_types().any(|mounted| mounted == *name))
}

inventory::submit! { Contribution::new(SUBSCRIBERS, "finance/payroll.people", Provenance { module: MODULE, source: file!(), line: line!() }, &Subscriber {
    events: &[
        On { event: RUN_APPROVED, versions: &[1] },
        On { event: CLAIM_APPROVED, versions: &[1] },
        On { event: CLAIM_REIMBURSED, versions: &[1] },
    ], handler: &People,
}) }

#[derive(Deserialize)]
struct RunApproved {
    company_id: i64,
    run_id: i64,
    reference: String,
    currency: String,
    pay_on: String,
    components: Vec<ComponentFact>,
}
#[derive(Deserialize)]
struct ComponentFact {
    code: String,
    direction: String,
    amount: String,
}
#[derive(Deserialize)]
struct ClaimApproved {
    company_id: i64,
    claim_id: i64,
    reference: String,
    currency: String,
    approved_amount: String,
    approval_date: String,
    category_code: String,
}

struct People;
impl Subscribe for People {
    fn handle<'a>(
        &'a self,
        tx: &'a Transaction<'_>,
        scope: &'a SystemScope,
        event: &'a Event,
    ) -> SubscriberFuture<'a> {
        Box::pin(async move {
            let tenant = scope.tenant();
            let company = event.payload["company_id"].as_i64().ok_or(Error::Invalid)?;
            // Serialize delivery with Finance posting. Archived companies retain
            // incoming history too; delivery has its own system authorization.
            tx.query_opt(
                "SELECT id FROM companies WHERE tenant_id=$1 AND id=$2 FOR UPDATE",
                &[&tenant, &company],
            )
            .await?
            .ok_or(Error::NotFound)?;
            let mut changes = Vec::new();
            match event.name.as_str() {
                RUN_APPROVED => {
                    let run: RunApproved = serde_json::from_value(event.payload.clone())
                        .map_err(|_| Error::Invalid)?;
                    let date: Date = run.pay_on.parse().map_err(|_| Error::Invalid)?;
                    let facts = json!({"run_id":run.run_id,"reference":run.reference,"currency_code":run.currency,"pay_date":date.to_string(),"event_key":event.key});
                    insert(
                        tx,
                        tenant,
                        run.company_id,
                        "source_run",
                        "finance_payroll_source_runs",
                        facts,
                        &mut changes,
                    )
                    .await?;
                    for component in run.components {
                        let amount = exact(&component.amount)?;
                        let facts = json!({"run_id":run.run_id,"code":component.code,"direction":component.direction,"amount":amount.to_string()});
                        insert(
                            tx,
                            tenant,
                            company,
                            "source_component",
                            "finance_payroll_source_components",
                            facts,
                            &mut changes,
                        )
                        .await?;
                    }
                    // The event receipt and these changes commit together. The
                    // run is immutable; another approval cannot replace it.
                }
                CLAIM_APPROVED | CLAIM_REIMBURSED => {
                    let claim: ClaimApproved = serde_json::from_value(event.payload.clone())
                        .map_err(|_| Error::Invalid)?;
                    let date: Date = claim.approval_date.parse().map_err(|_| Error::Invalid)?;
                    let amount = exact(&claim.approved_amount)?;
                    let old = tx.query_opt("SELECT to_jsonb(c) FROM finance_payroll_source_claims c WHERE tenant_id=$1 AND company_id=$2 AND claim_id=$3", &[&tenant,&company,&claim.claim_id]).await?.map(|r| r.get::<_,Value>(0));
                    let reimbursed = event.name == CLAIM_REIMBURSED;
                    if let Some(old) = old {
                        // Reimbursement adds standing, never changes the frozen
                        // approval facts (including when an approval is replayed).
                        if old["reference"] != claim.reference
                            || old["currency_code"] != claim.currency
                            || old["approved_on"] != date.to_string()
                            || old["category_code"] != claim.category_code
                            || exact(old["amount"].as_str().ok_or(Error::Internal)?)? != amount
                        {
                            return Err(Error::Invalid);
                        }
                        if reimbursed && old["reimbursed"] != true {
                            let new: Value = tx.query_one("UPDATE finance_payroll_source_claims SET reimbursed=true,event_key=$4 WHERE tenant_id=$1 AND company_id=$2 AND claim_id=$3 RETURNING to_jsonb(finance_payroll_source_claims.*)", &[&tenant,&company,&claim.claim_id,&event.key]).await?.get(0);
                            changes.push(store::change("source_claim", old, new));
                        }
                    } else {
                        insert(tx, tenant, claim.company_id, "source_claim", "finance_payroll_source_claims", json!({"claim_id":claim.claim_id,"reference":claim.reference,"currency_code":claim.currency,"approved_on":date.to_string(),"amount":amount.to_string(),"category_code":claim.category_code,"reimbursed":reimbursed,"event_key":event.key}), &mut changes).await?;
                    }
                }
                _ => return Err(Error::Invalid),
            }
            Ok(changes)
        })
    }
}

// Only private, fixed table names from the handler enter SQL. Decimal values
// stay text in the projection, preserving People's exact digits without a
// numeric-to-JSON floating-point conversion or PostgreSQL rounding.
async fn insert(
    tx: &Transaction<'_>,
    tenant: i64,
    company: i64,
    kind: &str,
    table: &str,
    facts: Value,
    changes: &mut Vec<Change>,
) -> bilibi_database::Result<Value> {
    let facts = facts.as_object().ok_or(Error::Internal)?;
    let columns = facts
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(",");
    let facts = json!(facts);
    let new: Value = tx.query_one(&format!("INSERT INTO {table} (tenant_id,company_id,{columns}) SELECT $1,$2,{columns} FROM jsonb_populate_record(NULL::{table},$3) RETURNING to_jsonb({table}.*)"), &[&tenant,&company,&facts]).await?.get(0);
    changes.push(store::change(kind, json!({}), new.clone()));
    Ok(new)
}
fn exact(value: &str) -> bilibi_database::Result<Decimal> {
    let amount = Decimal::from_str_exact(value).map_err(|_| Error::Invalid)?;
    if amount.is_sign_negative() {
        return Err(Error::Invalid);
    }
    Ok(amount)
}

pub(crate) struct Component {
    pub code: String,
    pub direction: String,
    pub amount: Decimal,
}
pub(crate) struct Run {
    pub id: i64,
    pub reference: String,
    pub currency_code: String,
    pub pay_date: Option<Date>,
    pub finalised: bool,
    pub components: Vec<Component>,
}
pub(crate) struct Claim {
    pub id: i64,
    pub reference: String,
    pub currency_code: String,
    pub approved_on: Option<Date>,
    pub amount: Decimal,
    pub category_code: Option<String>,
}

pub(crate) async fn runs(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
) -> Result<Vec<Value>> {
    Ok(client.query("SELECT jsonb_build_object('id',run_id,'reference',reference,'currency_code',currency_code,'pay_date',pay_date::text,'finalised',true,'status','approved') FROM finance_payroll_source_runs WHERE tenant_id=$1 AND company_id=$2 ORDER BY pay_date DESC,run_id DESC", &[&scope.tenant(),&company]).await?.iter().map(|r|r.get(0)).collect())
}
pub(crate) async fn run(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
    id: i64,
) -> Result<Run> {
    let row = client.query_opt("SELECT reference,currency_code,pay_date::text FROM finance_payroll_source_runs WHERE tenant_id=$1 AND company_id=$2 AND run_id=$3", &[&scope.tenant(),&company,&id]).await?.ok_or(PayrollError::SourceNotFound)?;
    let components = client.query("SELECT code,direction,amount FROM finance_payroll_source_components WHERE tenant_id=$1 AND company_id=$2 AND run_id=$3 ORDER BY code,direction", &[&scope.tenant(),&company,&id]).await?.iter().map(|r| Ok(Component {code:r.get(0),direction:r.get(1),amount:exact(r.get(2))?})).collect::<Result<Vec<_>>>()?;
    Ok(Run {
        id,
        reference: row.get(0),
        currency_code: row.get(1),
        pay_date: Some(
            row.get::<_, String>(2)
                .parse()
                .map_err(|_| Error::Internal)?,
        ),
        finalised: true,
        components,
    })
}
pub(crate) async fn claims(
    client: &(impl GenericClient + Sync),
    scope: &Scope,
    company: i64,
) -> Result<Vec<Claim>> {
    client.query("SELECT claim_id,reference,currency_code,approved_on::text,amount,category_code FROM finance_payroll_source_claims WHERE tenant_id=$1 AND company_id=$2 ORDER BY approved_on DESC,claim_id DESC", &[&scope.tenant(),&company]).await?.iter().map(|r| Ok(Claim {id:r.get(0),reference:r.get(1),currency_code:r.get(2),approved_on:Some(r.get::<_,String>(3).parse().map_err(|_| Error::Internal)?),amount:exact(r.get(4))?,category_code:Some(r.get(5))})).collect()
}
