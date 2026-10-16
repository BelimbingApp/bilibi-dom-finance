//! Controlled bookkeeping (Bilimbi `Ledger.Bookkeeping`): the company's
//! append-only setting changes, the latest deciding. A new company's book
//! opens controlled; the facade's `set_controlled_bookkeeping` and the
//! change log's reads follow in a later version of this module.
use bilibi_database::Result;
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

/// Whether the company is controlled by its latest change (`false` with none).
pub(crate) async fn controlled(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<bool> {
    let row = client
        .query_opt(
            "SELECT controlled FROM finance_ledger_bookkeeping_changes WHERE tenant_id=$1 AND company_id=$2 ORDER BY change_number DESC LIMIT 1",
            &[&tenant, &company_id],
        )
        .await?;
    Ok(row.is_some_and(|row| row.get(0)))
}

/// Appends a change of the setting. The database numbers and stamps it and
/// refuses one that changes nothing, so the caller decides that first with
/// [`controlled`]. Returns the row id and the change as audit records it.
pub(crate) async fn record(
    client: &impl GenericClient,
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
        }),
    ))
}
