//! Real contracts and immutable evidence, exercised around the existing
//! opening-register corrections and backdated settlement lifecycle.
use bilibi_agent_api::plans::{PlanAuthority, PlanIdentity, PlannedOperation, Plans};
use bilibi_database::Db;
use bilibi_finance_ledger::{Ledger, Money, NewEntry, NewLine};
use bilibi_finance_reports::Reports;
use bilibi_finance_walkthrough_fixture::sign_in;
use bilibi_module_registry::Services;
use bilibi_tenancy::{Authentication, ProofFuture};
use jiff::civil::date;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio_postgres::Transaction;
type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
struct Authority {
    authentication: Arc<Authentication>,
    user: i64,
}
impl PlanAuthority for Authority {
    fn verify<'a>(&'a self, tx: &'a Transaction<'_>) -> ProofFuture<'a, PlanIdentity> {
        Box::pin(async move {
            Ok(PlanIdentity {
                scope: self
                    .authentication
                    .authenticate(tx, 1, self.user, String::new())
                    .await?,
                credential_binding: format!("conversion-fixture:{}", self.user),
            })
        })
    }
}
pub struct Kept {
    pub id: i64,
    pub content: Value,
    pub pending: Value,
}
const COMPANY: i64 = 3;
const VIEWS: &[&str] = &[
    bilibi_finance_ledger::VIEW,
    bilibi_finance_attachments::VIEW,
    bilibi_finance_receivables::VIEW,
    bilibi_finance_payables::VIEW,
    bilibi_finance_reconciliation::VIEW,
    bilibi_finance_einvoice::VIEW,
    bilibi_finance_payroll::VIEW,
    bilibi_finance_sst::VIEW,
    bilibi_finance_fixed_assets::VIEW,
    bilibi_finance_reports::SIGN_OFF,
    "admin.company.tenant-wide.manage",
];
fn issue() -> PlannedOperation {
    PlannedOperation {
        key: "finance.reports.conversion.issue".into(),
        input: json!({"company_id":COMPANY,"as_of":"2026-08-01","reviewer_reference":"Source closing pack CONVERSION-3; reviewer workpaper C-109","note":"Review unresolved control postings and the external payroll and tax schedules. No historical completeness attestation."}),
    }
}
pub async fn before(db: &Db, services: &Services) -> Outcome<Kept> {
    let s = sign_in(db, services, 1, 4).await?;
    let auth = services.get::<Authentication>("base/tenancy.authentication")?;
    let authority = Authority {
        authentication: auth,
        user: 4,
    };
    let plans = services.get::<Plans>("base/agent_api.plans")?;
    let reports = services.get::<Reports>(bilibi_finance_reports::SERVICE)?;
    let preview = reports
        .conversion_report(&s, COMPANY, date(2026, 8, 1))
        .await?;
    assert!(preview["postings_by_owner"]["rows"]
        .as_array()
        .is_some_and(|a| !a.is_empty()));
    assert!(!preview["sources"]["receivables"]["opening_batch"]["rows"]
        .as_array()
        .unwrap()
        .is_empty());
    let op = [issue()];
    let review = plans.preview(&authority, &op).await?;
    assert_eq!(review["valid"], true, "{review}");
    let code = review["plan_code"].as_str().ok_or("no review code")?;
    let done = plans.commit(&s, &authority, code, &op).await?;
    let id = done["results"][0]["result"]["result"]["id"]
        .as_i64()
        .ok_or_else(|| format!("missing issued result: {done}"))?;
    assert!(plans.commit(&s, &authority, code, &op).await.is_err());
    let kept = reports.get_issued_report(&s, COMPANY, id).await?;
    assert_eq!(kept.kind, "conversion");
    assert_eq!(kept.content["accounts"], preview["accounts"]);
    let pending = plans.preview(&authority, &op).await?;
    assert_eq!(pending["valid"], true);
    let grants = services.get::<bilibi_authz::grants::Grants>("base/authz.grants")?;
    for cap in VIEWS {
        grants.set_direct(&s, 2, cap, Some(true)).await?;
    }
    let viewer = sign_in(db, services, 1, 2).await?;
    assert_eq!(
        reports
            .get_issued_report(&viewer, COMPANY, id)
            .await?
            .content,
        kept.content
    );
    grants
        .set_direct(&s, 2, bilibi_finance_payroll::VIEW, Some(false))
        .await?;
    assert!(reports
        .get_issued_report(&viewer, COMPANY, id)
        .await
        .is_err());
    assert!(reports
        .evidence_page(&viewer, COMPANY, "issued", Some(500), None)
        .await
        .is_err());
    grants
        .set_direct(&s, 2, bilibi_finance_payroll::VIEW, Some(true))
        .await?;
    let c = db.get().await?;
    for sql in [
        "UPDATE finance_reports_issued SET content='{}' WHERE id=$1",
        "DELETE FROM finance_reports_issued WHERE id=$1",
    ] {
        assert!(c.execute(sql, &[&id]).await.is_err());
    }
    println!("PASS conversion report: shared contract issuance, replay refusal, exact source/target facts, source-grant revocation and immutable issued copy");
    Ok(Kept {
        id,
        content: kept.content,
        pending,
    })
}
pub async fn after(db: &Db, services: &Services, kept: Kept) -> Outcome<()> {
    let s = sign_in(db, services, 1, 4).await?;
    let reports = services.get::<Reports>(bilibi_finance_reports::SERVICE)?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let authentication = services.get::<Authentication>("base/tenancy.authentication")?;
    let issuer = Authority {
        authentication: authentication.clone(),
        user: 4,
    };
    let plans = services.get::<Plans>("base/agent_api.plans")?;
    let before = reports.get_issued_report(&s, COMPANY, kept.id).await?;
    assert_eq!(
        before.content, kept.content,
        "register source corrections changed issued content"
    );
    let units = ledger.get_book(&s, COMPANY).await?.minor_units;
    let chart = ledger.list_accounts(&s, COMPANY).await?;
    let account = |code: &str| chart.iter().find(|a| a.code == code).unwrap().id;
    ledger
        .post_entry(
            &s,
            COMPANY,
            &NewEntry {
                entry_date: date(2026, 7, 1),
                description: "Unrelated backdated control posting after conversion issuance".into(),
                reference: None,
                owner: None,
                settlement_account_ids: vec![],
                lines: vec![
                    NewLine {
                        account_id: account("1100"),
                        debit: Money::parse("1", units)?,
                        credit: Money::ZERO,
                        description: None,
                    },
                    NewLine {
                        account_id: account("3000"),
                        debit: Money::ZERO,
                        credit: Money::parse("1", units)?,
                        description: None,
                    },
                ],
            },
        )
        .await?;
    assert_eq!(
        reports
            .get_issued_report(&s, COMPANY, kept.id)
            .await?
            .content,
        kept.content
    );
    let old = kept.pending["plan_code"].as_str().ok_or("no review code")?;
    assert!(
        plans.commit(&s, &issuer, old, &[issue()]).await.is_err(),
        "stale source review committed"
    );
    let current = reports
        .conversion_report(&s, COMPANY, date(2026, 8, 1))
        .await?;
    assert_ne!(
        current["through_entry_number"],
        kept.content["through_entry_number"]
    );
    assert_ne!(
        current["receivables"]["control"],
        kept.content["receivables"]["control"]
    );
    assert!(
        current["sources"]["receivables"]["opening_item"]["rows"]
            .as_array()
            .unwrap()
            .len()
            > 500,
        "evidence paging lost register rows"
    );
    let sign = [PlannedOperation {
        key: "finance.reports.conversion.sign".into(),
        input: json!({"company_id":COMPANY,"report_id":kept.id,"reviewer_reference":"Independent accountant review C-109/S","note":"Qualified: the retained report's differences and excluded history remain; external tax/payroll schedules require separate review."}),
    }];
    assert_eq!(
        plans.preview(&issuer, &sign).await?["valid"],
        false,
        "issuer signed own work"
    );
    let signer = Authority {
        authentication,
        user: 2,
    };
    let scope = sign_in(db, services, 1, 2).await?;
    let preview = plans.preview(&signer, &sign).await?;
    assert_eq!(preview["valid"], true, "{preview}");
    let done = plans
        .commit(
            &scope,
            &signer,
            preview["plan_code"].as_str().ok_or("no sign code")?,
            &sign,
        )
        .await?;
    let sign_id = done["results"][0]["result"]["result"]["id"]
        .as_i64()
        .ok_or("missing signoff")?;
    assert_eq!(
        plans.preview(&signer, &sign).await?["valid"],
        false,
        "second signoff accepted"
    );
    let evidence = reports
        .evidence_page(&scope, COMPANY, "conversion_signoff", Some(1), None)
        .await?;
    assert_eq!(evidence.rows[0]["report_id"], kept.id);
    let c = db.get().await?;
    assert!(c
        .execute(
            "DELETE FROM finance_reports_conversion_signoffs WHERE id=$1",
            &[&sign_id]
        )
        .await
        .is_err());
    assert!(c
        .batch_execute("TRUNCATE finance_reports_conversion_signoffs")
        .await
        .is_err());
    println!("PASS conversion report: original snapshot survives 10,000-row corrections and a backdated control posting; later reports retain differences and full evidence; stale review refused; independent qualified sign-off audited and immutable");
    Ok(())
}
