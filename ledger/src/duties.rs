//! The approval exceptions log (Bilimbi `Ledger.Duties` and
//! `Ledger.ApprovalException`): what an independent reviewer should read.
//! This version records the overrides of the ledger's own controls (a
//! reopened period or fiscal year, controlled bookkeeping turned off), with no
//! preparer; the separation-of-duties setting, `check_approval` and the
//! reviews follow in a later version of this module. The facade authorizes
//! and holds the company lock; these run inside it.
use bilibi_database::{Change, Result};
use serde_json::{json, Value};
use tokio_postgres::GenericClient;

const SOURCE: &str = "finance/ledger";
const SUBJECT_LIMIT: usize = 200;

/// An override of one of the ledger's controls, whoever made it.
pub(crate) struct Override<'a> {
    pub action: &'static str,
    pub subject_type: &'static str,
    pub subject_id: i64,
    pub subject: String,
    /// The control's own reason, already required and trimmed by its caller.
    pub reason: &'a str,
}

/// Appends the override; the database numbers and stamps it.
pub(crate) async fn record_override(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    actor: i64,
    impersonator: Option<i64>,
    exception: &Override<'_>,
) -> Result<Change> {
    let subject: String = exception
        .subject
        .trim()
        .chars()
        .take(SUBJECT_LIMIT)
        .collect();
    let row = client
        .query_one(
            "INSERT INTO finance_ledger_approval_exceptions(tenant_id,company_id,exception_number,source,action,\
             subject_type,subject_id,subject,same_person,override,prepared_by_user_id,prepared_by_impersonator_id,\
             approved_by_user_id,approved_by_impersonator_id,reason,separation_change_id,recorded_at) \
             VALUES($1,$2,NULL,$3,$4,$5,$6,$7,false,true,NULL,NULL,$8,$9,$10,NULL,NULL) \
             RETURNING id,exception_number",
            &[
                &tenant,
                &company_id,
                &SOURCE,
                &exception.action,
                &exception.subject_type,
                &exception.subject_id,
                &subject,
                &actor,
                &impersonator,
                &exception.reason,
            ],
        )
        .await?;
    Ok(Change {
        kind: "finance.ledger.approval_exception".into(),
        id: row.get(0),
        old: Value::Null,
        new: json!({
            "exception_number": row.get::<_, i64>(1),
            "source": SOURCE,
            "action": exception.action,
            "subject_type": exception.subject_type,
            "subject_id": exception.subject_id,
            "subject": subject,
            "same_person": false,
            "override": true,
            "approved_by_user_id": actor,
            "approved_by_impersonator_id": impersonator,
            "reason": exception.reason,
        }),
    })
}
