//! The export contract (Bilimbi `Reconciliation.Evidence`): one ID-ordered
//! page of a dataset per table this module owns, with explicit fields, for
//! the auditor's pack. An owned export contract, not permission to consume the
//! tables. Dates, times and amounts are written as text (an amount keeps all
//! its places), and nothing here is a credential.
use crate::{authorize, Reconciliation, ReconciliationError, Result, VIEW};
use bilibi_database::Error;
use serde::Serialize;
use serde_json::Value;

const DEFAULT_LIMIT: i64 = 500;
const ISO: &str = r#"YYYY-MM-DD"T"HH24:MI:SS.US"Z""#;

/// One page: the explicit fields, the rows and the cursor of the next page
/// (the last row's id when the page was full).
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Value>,
    pub next: Option<i64>,
}

#[derive(Clone, Copy)]
enum Kind {
    Plain,
    Date,
    Time,
    Amount,
}

struct Dataset {
    name: &'static str,
    table: &'static str,
    fields: &'static [(&'static str, Kind)],
}

use Kind::{Amount, Date, Plain, Time};

const ALL: &[Dataset] = &[
    Dataset {
        name: "opening_bridge",
        table: "finance_reconciliation_opening_bridges",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("bank_account_id", Plain),
            ("source_reference", Plain),
            ("statement_source", Plain),
            ("source_csv", Plain),
            ("review", Plain),
            ("conversion_date", Date),
            ("statement_balance", Amount),
            ("book_balance", Amount),
            ("opening_entry_id", Plain),
            ("opening_entry_number", Plain),
            ("item_count", Plain),
            ("replaces_bridge_id", Plain),
            ("reason", Plain),
            ("recorded_by_user_id", Plain),
            ("recorded_by_impersonator_id", Plain),
            ("recorded_at", Time),
        ],
    },
    Dataset {
        name: "opening_item",
        table: "finance_reconciliation_opening_items",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("bridge_id", Plain),
            ("source_id", Plain),
            ("item_date", Date),
            ("description", Plain),
            ("amount", Amount),
        ],
    },
    Dataset {
        name: "match_opening_item",
        table: "finance_reconciliation_match_opening_items",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("match_id", Plain),
            ("opening_item_id", Plain),
        ],
    },
    Dataset {
        name: "bank_account",
        table: "finance_reconciliation_bank_accounts",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("name", Plain),
            ("ledger_account_id", Plain),
            ("charges_account_id", Plain),
            ("interest_account_id", Plain),
            ("match_window_days", Plain),
            ("import_mapping", Plain),
            ("active", Plain),
            ("inserted_at", Time),
            ("updated_at", Time),
        ],
    },
    Dataset {
        name: "bank_account_change",
        table: "finance_reconciliation_bank_account_changes",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("bank_account_id", Plain),
            ("field", Plain),
            ("old_value", Plain),
            ("new_value", Plain),
            ("reason", Plain),
            ("changed_by_user_id", Plain),
            ("changed_by_impersonator_id", Plain),
            ("changed_at", Time),
        ],
    },
    Dataset {
        name: "match",
        table: "finance_reconciliation_matches",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("bank_account_id", Plain),
            ("match_number", Plain),
            ("note", Plain),
            ("matched_by_user_id", Plain),
            ("matched_by_impersonator_id", Plain),
            ("matched_at", Time),
        ],
    },
    Dataset {
        name: "match_entry",
        table: "finance_reconciliation_match_entries",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("match_id", Plain),
            ("bank_account_id", Plain),
            ("entry_id", Plain),
            ("entry_number", Plain),
            ("entry_date", Date),
            ("description", Plain),
            ("reference", Plain),
            ("amount", Amount),
        ],
    },
    Dataset {
        name: "match_event",
        table: "finance_reconciliation_match_events",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("event_number", Plain),
            ("match_id", Plain),
            ("action", Plain),
            ("reason", Plain),
            ("recorded_by_user_id", Plain),
            ("recorded_by_impersonator_id", Plain),
            ("recorded_at", Time),
        ],
    },
    Dataset {
        name: "match_line",
        table: "finance_reconciliation_match_lines",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("match_id", Plain),
            ("statement_line_id", Plain),
        ],
    },
    Dataset {
        name: "reconciliation_event",
        table: "finance_reconciliation_reconciliation_events",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("event_number", Plain),
            ("reconciliation_id", Plain),
            ("action", Plain),
            ("reason", Plain),
            ("book_balance", Amount),
            ("difference", Amount),
            ("through_entry_number", Plain),
            ("through_statement_number", Plain),
            ("through_match_event_number", Plain),
            ("through_opening_id", Plain),
            ("recorded_by_user_id", Plain),
            ("recorded_by_impersonator_id", Plain),
            ("recorded_at", Time),
        ],
    },
    Dataset {
        name: "reconciliation_item",
        table: "finance_reconciliation_reconciliation_items",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("reconciliation_id", Plain),
            ("item_number", Plain),
            ("kind", Plain),
            ("statement_line_id", Plain),
            ("entry_id", Plain),
            ("match_id", Plain),
            ("opening_item_id", Plain),
            ("item_date", Date),
            ("description", Plain),
            ("reference", Plain),
            ("amount", Amount),
        ],
    },
    Dataset {
        name: "reconciliation_record",
        table: "finance_reconciliation_reconciliations",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("bank_account_id", Plain),
            ("reconciliation_number", Plain),
            ("end_date", Date),
            ("statement_balance", Amount),
            ("opened_by_user_id", Plain),
            ("opened_by_impersonator_id", Plain),
            ("opened_at", Time),
        ],
    },
    Dataset {
        name: "statement",
        table: "finance_reconciliation_statements",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("bank_account_id", Plain),
            ("statement_number", Plain),
            ("file_name", Plain),
            ("file_sha256", Plain),
            ("import_mapping", Plain),
            ("line_count", Plain),
            ("start_date", Date),
            ("end_date", Date),
            ("opening_balance", Amount),
            ("closing_balance", Amount),
            ("imported_by_user_id", Plain),
            ("imported_by_impersonator_id", Plain),
            ("imported_at", Time),
        ],
    },
    Dataset {
        name: "statement_line",
        table: "finance_reconciliation_statement_lines",
        fields: &[
            ("id", Plain),
            ("tenant_id", Plain),
            ("company_id", Plain),
            ("statement_id", Plain),
            ("bank_account_id", Plain),
            ("line_number", Plain),
            ("line_date", Date),
            ("description", Plain),
            ("reference", Plain),
            ("amount", Amount),
            ("running_balance", Amount),
            ("fingerprint", Plain),
        ],
    },
];

/// The dataset names [`Reconciliation::evidence_page`] exports, sorted.
pub(crate) fn names() -> Vec<&'static str> {
    ALL.iter().map(|dataset| dataset.name).collect()
}

fn query(dataset: &Dataset) -> String {
    let select: Vec<String> = dataset
        .fields
        .iter()
        .map(|(field, kind)| match kind {
            Kind::Plain => format!("r.{field}"),
            Kind::Date => format!("to_char(r.{field},'YYYY-MM-DD') AS {field}"),
            Kind::Time => format!("to_char(r.{field},'{ISO}') AS {field}"),
            Kind::Amount => format!("r.{field}::text AS {field}"),
        })
        .collect();
    format!(
        "SELECT COALESCE(json_agg(t ORDER BY t.id),'[]'::json) FROM (SELECT {} FROM {} r \
         WHERE r.tenant_id = $1 AND r.company_id = $2 AND r.id > $3 ORDER BY r.id LIMIT $4) t",
        select.join(","),
        dataset.table
    )
}

impl Reconciliation {
    /// The datasets [`Reconciliation::evidence_page`] exports, one per table
    /// this module owns.
    pub fn evidence_datasets() -> Vec<&'static str> {
        names()
    }

    /// One ID-ordered evidence page with explicit fields, rows and next cursor
    /// (`finance.reconciliation.view`). `limit` is 1 to 500 (500 when absent)
    /// and `after` the last id of the previous page. Consume all pages in one
    /// repeatable-read transaction for a consistent export; individual pages
    /// are live reads.
    pub async fn evidence_page(
        &self,
        scope: &bilibi_tenancy::Scope,
        company_id: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        let client = self.db.get().await.map_err(Error::from)?;
        self.evidence_page_on(&**client, scope, company_id, dataset, limit, after)
            .await
    }

    /// Authorized evidence on the caller's snapshot; never opens another connection.
    pub async fn evidence_page_on(
        &self,
        client: &(impl tokio_postgres::GenericClient + Sync),
        scope: &bilibi_tenancy::Scope,
        company_id: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        authorize(client, scope, company_id, VIEW).await?;
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        let after = after.unwrap_or(0);
        let dataset = ALL
            .iter()
            .find(|candidate| candidate.name == dataset)
            .ok_or(ReconciliationError::InvalidOption)?;
        if !(1..=500).contains(&limit) || after < 0 {
            return Err(ReconciliationError::InvalidOption);
        }
        let row = client
            .query_one(
                &query(dataset),
                &[&scope.tenant(), &company_id, &after, &limit],
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
            fields: dataset.fields.iter().map(|(field, _)| *field).collect(),
            rows,
            next,
        })
    }
}
