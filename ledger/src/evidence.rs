//! The export contract (Bilimbi `Ledger.Evidence`): one ID-ordered page of
//! one of the ledger's tables with an explicit, frozen field list, for the
//! auditor's pack. An owned export contract, not permission to consume the
//! tables: a sibling reads it through [`crate::Ledger::evidence_page`] only.
//! Dates are `YYYY-MM-DD`, instants ISO 8601 UTC and amounts the column's
//! exact text (`100.0000`).
use crate::LedgerError;
use bilibi_database::Error;
use serde::Serialize;
use serde_json::Value;
use tokio_postgres::GenericClient;

const DEFAULT_LIMIT: i64 = 500;
const MAX_LIMIT: i64 = 500;
const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"";

/// Each dataset, its table and its fields, by dataset name. Freeze a field
/// list once exported; never add a credential to one.
const DATASETS: &[(&str, &str, &[&str])] = &[
    (
        "journal_approval",
        "finance_ledger_journal_approvals",
        &["id", "tenant_id", "company_id", "journal_id", "request_id"],
    ),
    (
        "opening_recognition_replacement",
        "finance_ledger_opening_recognition_replacements",
        &[
            "id",
            "tenant_id",
            "company_id",
            "recognition_id",
            "replaces_recognition_id",
        ],
    ),
    (
        "opening_recognition",
        "finance_ledger_opening_recognitions",
        &[
            "id",
            "tenant_id",
            "company_id",
            "entry_id",
            "account_id",
            "owner_module",
            "owner_record",
            "balance",
        ],
    ),
    (
        "account",
        "finance_ledger_accounts",
        &[
            "id",
            "tenant_id",
            "company_id",
            "code",
            "name",
            "account_type",
            "parent_id",
            "is_group",
            "active",
            "retained_earnings",
            "inserted_at",
            "updated_at",
        ],
    ),
    (
        "approval_exception",
        "finance_ledger_approval_exceptions",
        &[
            "id",
            "tenant_id",
            "company_id",
            "exception_number",
            "source",
            "action",
            "subject_type",
            "subject_id",
            "subject",
            "same_person",
            "override",
            "prepared_by_user_id",
            "prepared_by_impersonator_id",
            "approved_by_user_id",
            "approved_by_impersonator_id",
            "reason",
            "separation_change_id",
            "recorded_at",
        ],
    ),
    (
        "approval_exception_review",
        "finance_ledger_approval_exception_reviews",
        &[
            "id",
            "tenant_id",
            "company_id",
            "exception_id",
            "note",
            "reviewed_by_user_id",
            "reviewed_by_impersonator_id",
            "reviewed_at",
        ],
    ),
    (
        "book",
        "finance_ledger_books",
        &[
            "id",
            "tenant_id",
            "company_id",
            "currency_code",
            "minor_units",
            "inserted_at",
        ],
    ),
    (
        "bookkeeping_change",
        "finance_ledger_bookkeeping_changes",
        &[
            "id",
            "tenant_id",
            "company_id",
            "change_number",
            "controlled",
            "reason",
            "changed_by_user_id",
            "changed_by_impersonator_id",
            "changed_at",
        ],
    ),
    (
        "chart_template",
        "finance_ledger_chart_templates",
        &[
            "id",
            "tenant_id",
            "company_id",
            "key",
            "template",
            "inserted_at",
        ],
    ),
    (
        "closing_event",
        "finance_ledger_closing_events",
        &[
            "id",
            "tenant_id",
            "company_id",
            "event_number",
            "fiscal_year_id",
            "period_id",
            "action",
            "reason",
            "entry_id",
            "retained_earnings_account_id",
            "recorded_by_user_id",
            "recorded_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "duty_separation_change",
        "finance_ledger_duty_separation_changes",
        &[
            "id",
            "tenant_id",
            "company_id",
            "change_number",
            "required",
            "reason",
            "changed_by_user_id",
            "changed_by_impersonator_id",
            "changed_at",
        ],
    ),
    (
        "entry",
        "finance_ledger_entries",
        &[
            "id",
            "tenant_id",
            "company_id",
            "entry_number",
            "entry_date",
            "description",
            "reference",
            "currency_code",
            "reverses_entry_id",
            "owner_module",
            "owner_record",
            "posted_by_user_id",
            "posted_by_impersonator_id",
            "posted_at",
        ],
    ),
    (
        "fiscal_year",
        "finance_ledger_fiscal_years",
        &[
            "id",
            "tenant_id",
            "company_id",
            "start_date",
            "end_date",
            "opened_by_user_id",
            "opened_by_impersonator_id",
            "opened_at",
        ],
    ),
    (
        "journal",
        "finance_ledger_journals",
        &[
            "id",
            "tenant_id",
            "company_id",
            "journal_number",
            "entry_date",
            "description",
            "reference",
            "currency_code",
            "submitted_by_user_id",
            "submitted_by_impersonator_id",
            "submitted_at",
        ],
    ),
    (
        "journal_event",
        "finance_ledger_journal_events",
        &[
            "id",
            "tenant_id",
            "company_id",
            "journal_id",
            "event",
            "reason",
            "entry_id",
            "actor_user_id",
            "actor_impersonator_id",
            "occurred_at",
        ],
    ),
    (
        "journal_line",
        "finance_ledger_journal_lines",
        &[
            "id",
            "tenant_id",
            "company_id",
            "journal_id",
            "line_number",
            "account_id",
            "debit",
            "credit",
            "description",
        ],
    ),
    (
        "journal_replacement",
        "finance_ledger_journal_replacements",
        &[
            "id",
            "tenant_id",
            "company_id",
            "journal_id",
            "replaced_journal_id",
        ],
    ),
    (
        "late_recording_review",
        "finance_ledger_late_recording_reviews",
        &[
            "id",
            "tenant_id",
            "company_id",
            "entry_id",
            "reason",
            "reviewed_by_user_id",
            "reviewed_by_impersonator_id",
            "reviewed_at",
        ],
    ),
    (
        "line",
        "finance_ledger_entry_lines",
        &[
            "id",
            "tenant_id",
            "company_id",
            "entry_id",
            "line_number",
            "account_id",
            "debit",
            "credit",
            "description",
        ],
    ),
    (
        "opening_balances",
        "finance_ledger_opening_balances",
        &[
            "id",
            "tenant_id",
            "company_id",
            "source_reference",
            "source_csv",
            "journal_id",
        ],
    ),
    (
        "period",
        "finance_ledger_periods",
        &[
            "id",
            "tenant_id",
            "company_id",
            "fiscal_year_id",
            "period_number",
            "start_date",
            "end_date",
        ],
    ),
];

/// One page: the explicit fields, the rows and the cursor of the next page
/// (the last row's id when the page was full).
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Value>,
    pub next: Option<i64>,
}

/// Every dataset's name, sorted.
pub(crate) fn datasets() -> Vec<&'static str> {
    DATASETS.iter().map(|(name, _, _)| *name).collect()
}

/// A field as the export writes it.
fn column(field: &str) -> String {
    if field.ends_with("_at") {
        format!("to_char(r.{field},'{ISO}') AS {field}")
    } else if field.ends_with("_date") {
        format!("to_char(r.{field},'YYYY-MM-DD') AS {field}")
    } else if matches!(field, "debit" | "credit") {
        format!("r.{field}::text AS {field}")
    } else {
        format!("r.{field}")
    }
}

/// One page of `dataset` for the company: `InvalidOption` for an unknown
/// dataset, a limit outside 1 to 500, or a negative cursor.
pub(crate) async fn page(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    dataset: &str,
    limit: Option<i64>,
    after: Option<i64>,
) -> Result<EvidencePage, LedgerError> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let after = after.unwrap_or(0);
    let (_, table, fields) = DATASETS
        .iter()
        .find(|(name, _, _)| *name == dataset)
        .ok_or(LedgerError::InvalidOption)?;
    if !(1..=MAX_LIMIT).contains(&limit) || after < 0 {
        return Err(LedgerError::InvalidOption);
    }
    let select: Vec<String> = fields.iter().map(|field| column(field)).collect();
    let row = client
        .query_one(
            &format!(
                "SELECT COALESCE(json_agg(t ORDER BY t.id),'[]'::json) FROM (SELECT {} FROM {table} r \
                 WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id > $3 ORDER BY r.id LIMIT $4) t",
                select.join(",")
            ),
            &[&tenant, &company_id, &after, &limit],
        )
        .await
        .map_err(Error::from)?;
    let rows: Vec<Value> = match row.get::<_, Value>(0) {
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let next = (rows.len() as i64 == limit)
        .then(|| rows.last().and_then(|row| row["id"].as_i64()))
        .flatten();
    Ok(EvidencePage {
        fields: fields.to_vec(),
        rows,
        next,
    })
}
