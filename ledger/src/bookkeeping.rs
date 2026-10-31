//! Controlled bookkeeping (Bilimbi `Ledger.Bookkeeping` and
//! `Ledger.BookkeepingChange`): the company's append-only setting changes,
//! the latest deciding; with none, a company is not controlled. A new
//! company's book opens controlled. The facade authorizes and holds the
//! company lock; these run inside it.
use crate::{BookkeepingChange, FieldError};
use bilibi_agent_api::OperationClient;
use bilibi_database::Result;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

const REASON_LIMIT: usize = 500;
const COLUMNS: &str =
    "id,change_number,controlled,reason,changed_by_user_id,changed_by_impersonator_id,\
     to_char(changed_at,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')";

/// Whether the company is controlled by its latest change (`false` with none).
pub(crate) async fn controlled(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
) -> Result<bool> {
    Ok(latest(client, tenant, company_id)
        .await?
        .is_some_and(|change| change.controlled))
}

/// The company's latest change, or `None` before any.
pub(crate) async fn latest(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
) -> Result<Option<BookkeepingChange>> {
    let row = client
        .query_opt(
            &format!("SELECT {COLUMNS} FROM finance_ledger_bookkeeping_changes WHERE tenant_id=$1 AND company_id=$2 ORDER BY change_number DESC LIMIT 1"),
            &[&tenant, &company_id],
        )
        .await?;
    row.as_ref().map(from_row).transpose()
}

/// Every change of the company, oldest first.
pub(crate) async fn changes(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<BookkeepingChange>> {
    client
        .query(
            &format!("SELECT {COLUMNS} FROM finance_ledger_bookkeeping_changes WHERE tenant_id=$1 AND company_id=$2 ORDER BY change_number"),
            &[&tenant, &company_id],
        )
        .await?
        .iter()
        .map(from_row)
        .collect()
}

/// `BookkeepingChange.changeset/2`'s `reason`: trimmed, blank as none, at
/// most 500 characters, and required to turn controlled bookkeeping off.
pub(crate) fn validate(
    controlled: bool,
    reason: Option<&str>,
) -> std::result::Result<Option<String>, Vec<FieldError>> {
    let reason = reason
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned);
    match &reason {
        Some(text) if text.chars().count() > REASON_LIMIT => Err(vec![FieldError::new(
            "reason",
            format!("should be at most {REASON_LIMIT} character(s)"),
        )]),
        None if !controlled => Err(vec![FieldError::new(
            "reason",
            "is required to turn this off",
        )]),
        _ => Ok(reason),
    }
}

/// Appends a change of the setting. The database numbers and stamps it and
/// refuses one that changes nothing, so the caller decides that first with
/// [`controlled`]. Returns the row id and the change as audit records it.
pub(crate) async fn record(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    actor: i64,
    impersonator: Option<i64>,
    controlled: bool,
    reason: Option<&str>,
) -> Result<(i64, Value)> {
    // The BEFORE INSERT trigger numbers and stamps the row before NOT NULL is
    // checked, so the number and time are sent as NULL.
    let row = client
        .query_one(
            "INSERT INTO finance_ledger_bookkeeping_changes(tenant_id,company_id,change_number,controlled,reason,changed_by_user_id,changed_by_impersonator_id,changed_at) \
             VALUES($1,$2,NULL,$3,$4,$5,$6,NULL) RETURNING id,change_number",
            &[&tenant, &company_id, &controlled, &reason, &actor, &impersonator],
        )
        .await?;
    let id: i64 = row.get(0);
    let number: i64 = row.get(1);
    Ok((
        id,
        json!({
            "company_id": company_id,
            "change_number": number,
            "controlled": controlled,
            "reason": reason,
            "changed_by_user_id": actor,
            "changed_by_impersonator_id": impersonator,
        }),
    ))
}

fn from_row(row: &Row) -> Result<BookkeepingChange> {
    Ok(BookkeepingChange {
        id: row.get(0),
        change_number: row.get(1),
        controlled: row.get(2),
        reason: row.get(3),
        changed_by_user_id: row.get(4),
        changed_by_impersonator_id: row.get(5),
        changed_at: row
            .get::<_, String>(6)
            .parse()
            .map_err(|_| bilibi_database::Error::Internal)?,
    })
}
