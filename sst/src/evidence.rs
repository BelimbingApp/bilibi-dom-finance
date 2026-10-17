//! Bilimbi.Finance.Sst.Evidence: one ID-ordered page of one of this module's
//! tables with an explicit, frozen field list, for the auditor's pack. An
//! owned export contract, not permission to consume the tables: a sibling
//! reads it through [`Sst::evidence_page`] only. Dates are `YYYY-MM-DD`,
//! instants ISO 8601 UTC and amounts and rates the column's exact text.
use crate::*;
use serde_json::Value;

/// Each dataset, its table and its fields, sorted by dataset name. Freeze a
/// field list once exported; never add a credential to one.
const DATASETS: &[(&str, &str, &[&str])] = &[
    (
        "adjustment",
        "finance_sst_return_adjustments",
        &[
            "id",
            "tenant_id",
            "company_id",
            "adjustment_number",
            "return_id",
            "action",
            "withdraws_adjustment_id",
            "copied_from_adjustment_id",
            "box",
            "taxable_amount",
            "tax_amount",
            "description",
            "reference",
            "recorded_by_user_id",
            "recorded_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "mapping",
        "finance_sst_mappings",
        &[
            "id",
            "tenant_id",
            "company_id",
            "mapping_number",
            "source",
            "tax_code_id",
            "tax_code",
            "box",
            "reason",
            "recorded_by_user_id",
            "recorded_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "registration",
        "finance_sst_registrations",
        &[
            "id",
            "tenant_id",
            "company_id",
            "version",
            "sales_tax_number",
            "service_tax_number",
            "period_months",
            "period_start",
            "service_tax_basis",
            "reason",
            "recorded_by_user_id",
            "recorded_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "return",
        "finance_sst_returns",
        &[
            "id",
            "tenant_id",
            "company_id",
            "return_number",
            "tax_type",
            "registration_id",
            "period_start",
            "period_end",
            "due_date",
            "amends_return_id",
            "reason",
            "opened_by_user_id",
            "opened_by_impersonator_id",
            "opened_at",
        ],
    ),
    (
        "return_account",
        "finance_sst_return_accounts",
        &[
            "id",
            "tenant_id",
            "company_id",
            "return_id",
            "line_number",
            "account_id",
            "account_code",
            "account_name",
            "ledger_debits",
            "ledger_credits",
            "document_debits",
            "document_credits",
            "difference",
        ],
    ),
    (
        "return_box",
        "finance_sst_return_boxes",
        &[
            "id",
            "tenant_id",
            "company_id",
            "return_id",
            "line_number",
            "box",
            "taxable_amount",
            "tax_amount",
        ],
    ),
    (
        "return_event",
        "finance_sst_return_events",
        &[
            "id",
            "tenant_id",
            "company_id",
            "event_number",
            "return_id",
            "action",
            "reason",
            "version",
            "through_entry_number",
            "through_mapping_number",
            "through_adjustment_number",
            "currency_code",
            "minor_units",
            "tax_payable",
            "recorded_by_user_id",
            "recorded_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "return_source",
        "finance_sst_return_sources",
        &[
            "id",
            "tenant_id",
            "company_id",
            "return_id",
            "line_number",
            "source",
            "document_kind",
            "measure",
            "prior_period",
            "tax_code_id",
            "tax_code",
            "rate",
            "account_id",
            "box",
            "mapping_number",
            "document_count",
            "taxable_amount",
            "tax_amount",
        ],
    ),
];

/// Positional rows in exactly the order of `fields`; `next` is the last id
/// when the page is full, even if the following page is empty.
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Vec<Value>>,
    pub next: Option<i64>,
}

fn column(field: &str) -> String {
    match field {
        f if f.ends_with("_at") => {
            format!("to_char(r.{f},'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"')")
        }
        "period_start" | "period_end" | "due_date" => format!("to_char(r.{field},'YYYY-MM-DD')"),
        "taxable_amount" | "tax_amount" | "tax_payable" | "rate" | "ledger_debits"
        | "ledger_credits" | "document_debits" | "document_credits" | "difference" => {
            format!("r.{field}::text")
        }
        f => format!("r.{f}"),
    }
}

impl Sst {
    /// Every evidence dataset this module exports, sorted by name.
    pub fn evidence_datasets() -> Vec<&'static str> {
        DATASETS.iter().map(|(name, _, _)| *name).collect()
    }

    /// One live, ID-ordered page of `dataset` (`finance.sst.view`). Separate
    /// pages share no snapshot: read every page in one repeatable-read
    /// transaction for a consistent export. `limit` is 1 to 500 (default
    /// 500) and `after` the last id read (default 0); an unknown dataset or
    /// option is `InvalidOption`, after authorization.
    pub async fn evidence_page(
        &self,
        s: &Scope,
        company: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        let c = self.db.get().await.map_err(Error::from)?;
        self.evidence_page_on(&**c, s, company, dataset, limit, after)
            .await
    }

    /// Authorized evidence on the caller's snapshot; never opens another connection.
    pub async fn evidence_page_on(
        &self,
        c: &(impl tokio_postgres::GenericClient + Sync),
        s: &Scope,
        company: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        authorize(c, s, company, VIEW).await?;
        let limit = limit.unwrap_or(500);
        let after = after.unwrap_or(0);
        let (_, table, fields) = DATASETS
            .iter()
            .find(|(name, _, _)| *name == dataset)
            .ok_or(SstError::InvalidOption)?;
        if !(1..=500).contains(&limit) || after < 0 {
            return Err(SstError::InvalidOption);
        }
        // Every identifier comes from the private frozen catalog above.
        let select = fields
            .iter()
            .map(|f| column(f))
            .collect::<Vec<_>>()
            .join(",");
        let rows: Vec<Vec<Value>> = c
            .query(
                &format!("SELECT json_build_array({select}) FROM {table} r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id>$3 ORDER BY r.id LIMIT $4"),
                &[&s.tenant(), &company, &after, &limit],
            )
            .await?
            .iter()
            .map(|r| match r.get::<_, Value>(0) {
                Value::Array(values) => values,
                other => vec![other],
            })
            .collect();
        let next = (rows.len() as i64 == limit)
            .then(|| rows.last().and_then(|r| r.first()).and_then(Value::as_i64))
            .flatten();
        Ok(EvidencePage {
            fields: fields.to_vec(),
            rows,
            next,
        })
    }
}
