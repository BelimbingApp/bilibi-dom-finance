//! Private Payroll row reads and audited event writes (Bilimbi Payroll.Events).
use crate::{Attempt, PayrollError, Posting, Result, ReversalRecord};
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_finance_ledger::{Entry, Money};
use bilibi_tenancy::Scope;
use serde_json::Value;
use tokio_postgres::GenericClient;

pub(crate) const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"";

pub(crate) fn change(kind: &str, old: Value, new: Value) -> Change {
    Change {
        kind: format!("finance.payroll.{kind}"),
        id: new["id"].as_i64().expect("stored row id"),
        old,
        new,
    }
}

pub(crate) async fn settings(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
) -> Result<Option<Value>> {
    Ok(client.query_opt("SELECT to_jsonb(s) FROM finance_payroll_settings s WHERE tenant_id=$1 AND company_id=$2", &[&tenant, &company]).await?.map(|r| r.get(0)))
}
pub(crate) async fn mappings(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
) -> Result<Vec<Value>> {
    Ok(client.query("SELECT to_jsonb(m) FROM finance_payroll_mappings m WHERE tenant_id=$1 AND company_id=$2 ORDER BY kind,code,direction", &[&tenant, &company]).await?.iter().map(|r| r.get(0)).collect())
}
pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| PayrollError::Failed(Error::Internal))
}

async fn events(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    posting: Option<i64>,
) -> Result<Vec<Value>> {
    Ok(client.query(&format!(
        "SELECT to_jsonb(e) || jsonb_build_object('total',e.total::text,'occurred_at',to_char(e.occurred_at,'{ISO}')) \
         FROM finance_payroll_posting_events e WHERE tenant_id=$1 AND company_id=$2 \
         AND ($3::bigint IS NULL OR (source_kind,source_id) = \
         (SELECT source_kind,source_id FROM finance_payroll_posting_events WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND event='posted')) \
         ORDER BY id"), &[&tenant, &company, &posting]).await?.iter().map(|r| r.get(0)).collect())
}

fn view(post: &Value, events: &[Value]) -> Result<Posting> {
    let same =
        |e: &&Value| e["source_kind"] == post["source_kind"] && e["source_id"] == post["source_id"];
    let reversal = events
        .iter()
        .filter(same)
        .find(|e| e["event"] == "reversed");
    let latest_success = reversal.unwrap_or(post)["id"]
        .as_i64()
        .ok_or(Error::Internal)?;
    let failure = events.iter().filter(same).rev().find(|e| {
        e["id"].as_i64().unwrap_or(0) > latest_success
            && matches!(
                e["event"].as_str(),
                Some("posting_failed" | "reversal_failed")
            )
    });
    Ok(Posting {
        id: decode(post["id"].clone())?,
        source_kind: decode(post["source_kind"].clone())?,
        source_id: decode(post["source_id"].clone())?,
        source_reference: decode(post["source_reference"].clone())?,
        entry_id: decode(post["entry_id"].clone())?,
        entry_number: decode(post["entry_number"].clone())?,
        entry_date: decode(post["entry_date"].clone())?,
        total: decode(post["total"].clone())?,
        currency_code: decode(post["currency_code"].clone())?,
        posted_by_user_id: decode(post["actor_user_id"].clone())?,
        posted_by_impersonator_id: decode(post["actor_impersonator_id"].clone())?,
        posted_at: decode(post["occurred_at"].clone())?,
        reversal: reversal
            .map(|r| -> Result<_> {
                Ok(ReversalRecord {
                    entry_id: decode(r["entry_id"].clone())?,
                    entry_number: decode(r["entry_number"].clone())?,
                    entry_date: decode(r["entry_date"].clone())?,
                    reason: decode(r["reason"].clone())?,
                    reversed_by_user_id: decode(r["actor_user_id"].clone())?,
                    reversed_by_impersonator_id: decode(r["actor_impersonator_id"].clone())?,
                    reversed_at: decode(r["occurred_at"].clone())?,
                })
            })
            .transpose()?,
        last_failure: failure
            .map(|r| -> Result<_> {
                Ok(Attempt {
                    action: if r["event"] == "posting_failed" {
                        "post"
                    } else {
                        "reverse"
                    }
                    .into(),
                    reason: decode(r["reason"].clone())?,
                    actor_user_id: decode(r["actor_user_id"].clone())?,
                    occurred_at: decode(r["occurred_at"].clone())?,
                })
            })
            .transpose()?,
    })
}
pub(crate) async fn postings(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
) -> Result<Vec<Posting>> {
    // Bound the operational list; complete history is paged through evidence.
    let ids = client.query("SELECT id FROM finance_payroll_posting_events WHERE tenant_id=$1 AND company_id=$2 AND event='posted' ORDER BY id DESC LIMIT 500", &[&tenant, &company]).await?;
    let mut result = Vec::new();
    for id in ids {
        result.push(posting(client, tenant, company, id.get(0)).await?);
    }
    Ok(result)
}
pub(crate) async fn posting(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    id: i64,
) -> Result<Posting> {
    let rows = events(client, tenant, company, Some(id)).await?;
    let post = rows
        .iter()
        .find(|e| e["id"].as_i64() == Some(id) && e["event"] == "posted")
        .ok_or(PayrollError::PostingNotFound)?;
    view(post, &rows)
}
pub(crate) async fn already_posted(
    client: &impl GenericClient,
    tenant: i64,
    company: i64,
    kind: &str,
    id: i64,
) -> Result<bool> {
    Ok(client.query_opt("SELECT 1 FROM finance_payroll_posting_events WHERE tenant_id=$1 AND company_id=$2 AND source_kind=$3 AND source_id=$4 AND event='posted'", &[&tenant, &company, &kind, &id]).await?.is_some())
}

/// Shape guarantees: a success has an entry; a failure cannot carry one.
pub(crate) enum Outcome<'a> {
    Posted(&'a Entry, Money),
    Reversed(&'a Entry, Money, &'a str),
    Failed { reverse: bool, reason: &'a str },
}
pub(crate) struct Source<'a> {
    pub kind: &'a str,
    pub id: i64,
    pub reference: &'a str,
}
pub(crate) async fn record(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    source: Source<'_>,
    outcome: Outcome<'_>,
) -> Result<i64> {
    let (event, entry, total, reason) = match outcome {
        Outcome::Posted(entry, total) => ("posted", Some(entry), Some(total.amount()), None),
        Outcome::Reversed(entry, total, reason) => {
            ("reversed", Some(entry), Some(total.amount()), Some(reason))
        }
        Outcome::Failed { reverse, reason } => (
            if reverse {
                "reversal_failed"
            } else {
                "posting_failed"
            },
            None,
            None,
            Some(reason),
        ),
    };
    let tenant = scope.tenant();
    let actor = scope.user();
    let impersonator = scope.impersonator().map(|p| p.user());
    let entry_id = entry.map(|e| e.id);
    let entry_number = entry.map(|e| e.entry_number);
    let entry_date = entry.map(|e| e.entry_date.to_string());
    let currency = entry.map(|e| e.currency_code.as_str());
    let reference: String = source.reference.chars().take(100).collect();
    let kind = source.kind.to_owned();
    let id = source.id;
    let reason = reason.map(str::to_owned);
    let currency = currency.map(str::to_owned);
    Ok(tx.mutate(move |sql| Box::pin(async move {
        let row = sql.query_one("INSERT INTO finance_payroll_posting_events AS e (tenant_id,company_id,source_kind,source_id,source_reference,event,entry_id,entry_number,entry_date,total,currency_code,reason,actor_user_id,actor_impersonator_id,occurred_at) \
            VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9::text::date,$10,$11,$12,$13,$14,NULL) RETURNING to_jsonb(e)",
            &[&tenant,&company,&kind,&id,&reference,&event,&entry_id,&entry_number,&entry_date,&total,&currency,&reason,&actor,&impersonator]).await?;
        let value: Value = row.get(0);
        Ok((value["id"].as_i64().ok_or(Error::Internal)?, vec![change("posting_event", Value::Null, value)]))
    })).await?)
}
