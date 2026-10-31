//! Entries recorded late and their independent reviews (Bilimbi
//! `Ledger.LateRecordings` and `Ledger.LateRecordingReview`). An entry's days
//! are whole UTC days from its entry date (its transaction date) to the day
//! the database stamped it. A year-end closing entry and its reopen reversal
//! are dated the year's end by the ledger itself, so they are never listed.
//! The facade authorizes and holds the company lock; these run inside it.
use crate::{
    periods::NOT_CLOSING, FieldError, LateRecording, LateRecordingQuery, LateRecordingReview,
    LateStatus, LedgerError,
};
use bilibi_agent_api::OperationClient;
use bilibi_database::{Change, Result};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

/// From this many days an entry is listed.
pub(crate) const APPROACHING_DAYS: i32 = 45;
/// Past this many days it is late.
pub(crate) const LATE_DAYS: i32 = 60;
const REASON_LIMIT: usize = 500;
const DEFAULT_LIMIT: u32 = 100;
const MAX_LIMIT: u32 = 500;

const STAMP: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'";

/// A posted entry of the company that is not a closing entry: its days and
/// posters, and whether it has a review.
pub(crate) struct Recorded {
    pub days: i32,
    pub posted_by_user_id: i64,
    pub posted_by_impersonator_id: Option<i64>,
    pub reviewed: bool,
}

pub(crate) async fn entry(
    client: &dyn OperationClient,
    tenant: i64,
    company_id: i64,
    entry_id: i64,
) -> Result<Option<Recorded>> {
    let row = client
        .query_opt(
            &format!(
                "SELECT CAST(e.posted_at AS date) - e.entry_date, e.posted_by_user_id, e.posted_by_impersonator_id, \
                 EXISTS (SELECT 1 FROM finance_ledger_late_recording_reviews r WHERE r.entry_id = e.id) \
                 FROM finance_ledger_entries e WHERE e.tenant_id=$1 AND e.company_id=$2 AND e.id=$3 AND {NOT_CLOSING}"
            ),
            &[&tenant, &company_id, &entry_id],
        )
        .await?;
    Ok(row.map(|row| Recorded {
        days: row.get(0),
        posted_by_user_id: row.get(1),
        posted_by_impersonator_id: row.get(2),
        reviewed: row.get(3),
    }))
}

/// Shared refusal checks for the facade and reviewed operation.
pub(crate) async fn prepare(
    client: &dyn OperationClient,
    scope: &Scope,
    company: i64,
    entry_id: i64,
    reason: &str,
) -> std::result::Result<String, LedgerError> {
    let entry = entry(client, scope.tenant(), company, entry_id)
        .await?
        .ok_or(LedgerError::EntryNotFound)?;
    if entry.days < APPROACHING_DAYS {
        return Err(LedgerError::NotLateRecorded);
    }
    if entry.reviewed {
        return Err(LedgerError::AlreadyReviewed);
    }
    if crate::same_person(
        crate::people(scope),
        (entry.posted_by_user_id, entry.posted_by_impersonator_id),
    ) {
        return Err(LedgerError::ReviewerNotIndependent);
    }
    validate(reason).map_err(LedgerError::Invalid)
}

/// Facts kept with the entry's one record page and its review preview.
pub(crate) async fn facts(
    client: &dyn OperationClient,
    tenant: i64,
    company: i64,
    entry_id: i64,
) -> Result<Value> {
    let Some(recorded) = entry(client, tenant, company, entry_id).await? else {
        return Ok(Value::Null);
    };
    if recorded.days < APPROACHING_DAYS {
        return Ok(Value::Null);
    }
    let review = client.query_opt("SELECT id,reason,reviewed_by_user_id,reviewed_by_impersonator_id,to_char(reviewed_at,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') FROM finance_ledger_late_recording_reviews WHERE tenant_id=$1 AND company_id=$2 AND entry_id=$3", &[&tenant,&company,&entry_id]).await?.map(|r| json!({"id":r.get::<_,i64>(0),"reason":r.get::<_,String>(1),"reviewed_by_user_id":r.get::<_,i64>(2),"reviewed_by_impersonator_id":r.get::<_,Option<i64>>(3),"reviewed_at":r.get::<_,String>(4)}));
    Ok(json!({"days":recorded.days,"status":status(recorded.days),"review":review}))
}

/// The listed entries, oldest number first, each with its review or none.
pub(crate) async fn list(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    query: &LateRecordingQuery,
) -> std::result::Result<Vec<LateRecording>, LedgerError> {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) || query.after.is_some_and(|after| after < 0) {
        return Err(LedgerError::InvalidOption);
    }
    let (low, high) = match query.status {
        None => (APPROACHING_DAYS, i32::MAX),
        Some(LateStatus::Approaching) => (APPROACHING_DAYS, LATE_DAYS),
        Some(LateStatus::Late) => (LATE_DAYS + 1, i32::MAX),
    };
    let rows = client
        .query(
            &format!(
                "SELECT e.id, e.entry_number, e.entry_date::text, to_char(e.posted_at,{STAMP}), \
                 CAST(e.posted_at AS date) - e.entry_date, e.description, e.reference, \
                 e.posted_by_user_id, e.posted_by_impersonator_id, \
                 r.id, r.reason, r.reviewed_by_user_id, r.reviewed_by_impersonator_id, to_char(r.reviewed_at,{STAMP}) \
                 FROM finance_ledger_entries e \
                 LEFT JOIN finance_ledger_late_recording_reviews r ON r.entry_id = e.id \
                 WHERE e.tenant_id=$1 AND e.company_id=$2 AND {NOT_CLOSING} \
                 AND CAST(e.posted_at AS date) - e.entry_date BETWEEN $3 AND $4 \
                 AND ($5::boolean IS NULL OR (r.id IS NOT NULL) = $5) \
                 AND ($6::bigint IS NULL OR e.entry_number > $6) \
                 ORDER BY e.entry_number LIMIT $7"
            ),
            &[
                &tenant,
                &company_id,
                &low,
                &high,
                &query.reviewed,
                &query.after,
                &i64::from(limit),
            ],
        )
        .await
        .map_err(bilibi_database::Error::from)?;
    Ok(rows.iter().map(from_row).collect::<Result<Vec<_>>>()?)
}

/// `LateRecordingReview.changeset/2`'s `reason`: trimmed, required, at most
/// 500 characters.
pub(crate) fn validate(reason: &str) -> std::result::Result<String, Vec<FieldError>> {
    let reason = reason.trim();
    if reason.is_empty() {
        Err(vec![FieldError::new("reason", "can't be blank")])
    } else if reason.chars().count() > REASON_LIMIT {
        Err(vec![FieldError::new(
            "reason",
            format!("should be at most {REASON_LIMIT} character(s)"),
        )])
    } else {
        Ok(reason.to_owned())
    }
}

/// Appends the review; the database stamps it and refuses an entry recorded
/// less than 45 days late, a second review and a reviewer who overlaps the
/// poster.
pub(crate) async fn insert_review(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    entry_id: i64,
    actor: i64,
    impersonator: Option<i64>,
    reason: &str,
) -> Result<(LateRecordingReview, Change)> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO finance_ledger_late_recording_reviews(tenant_id,company_id,entry_id,reason,\
                 reviewed_by_user_id,reviewed_by_impersonator_id,reviewed_at) \
                 VALUES($1,$2,$3,$4,$5,$6,NULL) RETURNING id,to_char(reviewed_at,{STAMP})"
            ),
            &[&tenant, &company_id, &entry_id, &reason, &actor, &impersonator],
        )
        .await?;
    let review = LateRecordingReview {
        id: row.get(0),
        entry_id,
        reason: reason.to_owned(),
        reviewed_by_user_id: actor,
        reviewed_by_impersonator_id: impersonator,
        reviewed_at: parse(row.get(1))?,
    };
    let change = Change {
        kind: "finance.ledger.late_recording_review".into(),
        id: review.id,
        old: Value::Null,
        new: json!({
            "entry_id": entry_id, "reason": reason, "reviewed_by_user_id": actor,
            "reviewed_by_impersonator_id": impersonator,
            "reviewed_at": review.reviewed_at.to_string(),
        }),
    };
    Ok((review, change))
}

/// `"late"` past the late threshold, `"approaching"` otherwise.
pub(crate) fn status(days: i32) -> LateStatus {
    if days > LATE_DAYS {
        LateStatus::Late
    } else {
        LateStatus::Approaching
    }
}

fn from_row(row: &Row) -> Result<LateRecording> {
    let days: i32 = row.get(4);
    let review = match row.get::<_, Option<i64>>(9) {
        Some(id) => Some(LateRecordingReview {
            id,
            entry_id: row.get(0),
            reason: row.get(10),
            reviewed_by_user_id: row.get(11),
            reviewed_by_impersonator_id: row.get(12),
            reviewed_at: parse(row.get(13))?,
        }),
        None => None,
    };
    Ok(LateRecording {
        entry_id: row.get(0),
        entry_number: row.get(1),
        entry_date: parse(row.get(2))?,
        posted_at: parse(row.get(3))?,
        days,
        status: status(days),
        description: row.get(5),
        reference: row.get(6),
        posted_by_user_id: row.get(7),
        posted_by_impersonator_id: row.get(8),
        review,
    })
}

fn parse<T: std::str::FromStr>(text: String) -> Result<T> {
    text.parse().map_err(|_| bilibi_database::Error::Internal)
}
