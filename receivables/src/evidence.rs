//! Frozen auditor export mirroring Bilimbi `Finance.Receivables.Evidence`.
//! Dates and UTC instants follow Ledger.Evidence; decimals retain exact SQL text.
use crate::{authorize, ReceivableError, Receivables, Result, VIEW};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
use serde::Serialize;
use serde_json::Value;

const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"";
const DATASETS: &[(&str, &str, &[&str])] = &[
    (
        "allocation",
        "finance_receivables_allocations",
        &[
            "id",
            "tenant_id",
            "company_id",
            "payment_id",
            "document_id",
            "amount",
        ],
    ),
    (
        "customer",
        "finance_receivables_customers",
        &[
            "id",
            "tenant_id",
            "company_id",
            "code",
            "name",
            "tax_identifier",
            "registration_identifier",
            "email",
            "phone",
            "line1",
            "line2",
            "line3",
            "locality",
            "postcode",
            "admin1_code",
            "country_iso",
            "payment_terms_days",
            "active",
            "inserted_at",
            "updated_at",
        ],
    ),
    (
        "document",
        "finance_receivables_documents",
        &[
            "id",
            "tenant_id",
            "company_id",
            "kind",
            "status",
            "series_id",
            "customer_id",
            "credited_document_id",
            "receivable_account_id",
            "currency_code",
            "issue_date",
            "due_date",
            "description",
            "subtotal",
            "tax_total",
            "total",
            "document_number",
            "document_reference",
            "party_name",
            "party_tax_identifier",
            "party_registration_identifier",
            "party_email",
            "party_phone",
            "party_line1",
            "party_line2",
            "party_line3",
            "party_locality",
            "party_postcode",
            "party_admin1_code",
            "party_country_iso",
            "ledger_entry_id",
            "ledger_entry_number",
            "issued_by_user_id",
            "issued_by_impersonator_id",
            "issued_at",
            "discarded_by_user_id",
            "discarded_at",
            "inserted_at",
            "updated_at",
        ],
    ),
    // Bilimbi's frozen document_line list repeats quantity. Keep both positions,
    // including both values in each positional row, rather than deduplicating it.
    (
        "document_line",
        "finance_receivables_document_lines",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "line_number",
            "description",
            "quantity",
            "unit_price",
            "amount",
            "account_id",
            "tax_code_id",
            "tax_rate",
            "tax_amount",
            "quantity",
        ],
    ),
    (
        "payment",
        "finance_receivables_payments",
        &[
            "id",
            "tenant_id",
            "company_id",
            "series_id",
            "customer_id",
            "payment_number",
            "payment_reference",
            "payment_date",
            "amount",
            "currency_code",
            "deposit_account_id",
            "reference",
            "description",
            "ledger_entry_id",
            "ledger_entry_number",
            "recorded_by_user_id",
            "recorded_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "payment_reversal",
        "finance_receivables_payment_reversals",
        &[
            "id",
            "tenant_id",
            "company_id",
            "payment_id",
            "reversal_date",
            "reason",
            "ledger_entry_id",
            "ledger_entry_number",
            "reversed_by_user_id",
            "reversed_by_impersonator_id",
            "reversed_at",
        ],
    ),
    (
        "series",
        "finance_receivables_series",
        &[
            "id",
            "tenant_id",
            "company_id",
            "code",
            "name",
            "kind",
            "prefix",
            "number_width",
            "active",
            "inserted_at",
            "updated_at",
        ],
    ),
    (
        "setup",
        "finance_receivables_setups",
        &[
            "id",
            "tenant_id",
            "company_id",
            "receivable_account_id",
            "income_account_id",
            "inserted_at",
            "updated_at",
        ],
    ),
    (
        "setup_change",
        "finance_receivables_setup_changes",
        &[
            "id",
            "tenant_id",
            "company_id",
            "field",
            "old_value",
            "new_value",
            "changed_by_user_id",
            "changed_by_impersonator_id",
            "changed_at",
        ],
    ),
    (
        "tax_code",
        "finance_receivables_tax_codes",
        &[
            "id",
            "tenant_id",
            "company_id",
            "code",
            "name",
            "rate",
            "account_id",
            "active",
            "inserted_at",
            "updated_at",
        ],
    ),
    (
        "tax_line",
        "finance_receivables_document_tax_lines",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "tax_code_id",
            "tax_code",
            "tax_rate",
            "taxable_amount",
            "tax_amount",
            "account_id",
        ],
    ),
];

/// Positional rows in precisely the order of `fields`; `next` is the last ID
/// when the page is full, even if a subsequent page will be empty.
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Vec<Value>>,
    pub next: Option<i64>,
}

fn column(field: &str) -> String {
    if field.ends_with("_at") {
        format!("to_char(r.{field},'{ISO}')")
    } else if field.ends_with("_date") {
        format!("to_char(r.{field},'YYYY-MM-DD')")
    } else if matches!(
        field,
        "quantity"
            | "unit_price"
            | "amount"
            | "rate"
            | "tax_rate"
            | "tax_amount"
            | "taxable_amount"
            | "subtotal"
            | "tax_total"
            | "total"
    ) {
        format!("r.{field}::text")
    } else {
        format!("r.{field}")
    }
}

impl Receivables {
    /// Every owned evidence dataset, sorted by name.
    pub fn evidence_datasets() -> Vec<&'static str> {
        DATASETS.iter().map(|(name, _, _)| *name).collect()
    }

    /// A live, ID-ordered page requiring receivables VIEW. Separate pages do
    /// not share a snapshot. Limit defaults to 500 and must be 1..=500; after
    /// defaults to zero and must be nonnegative. Unknown datasets and invalid
    /// options return `InvalidOption`, after authorization.
    pub async fn evidence_page(
        &self,
        scope: &Scope,
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
        scope: &Scope,
        company_id: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        authorize(client, scope, company_id, VIEW).await?;
        let limit = limit.unwrap_or(500);
        let after = after.unwrap_or(0);
        let (_, table, fields) = DATASETS
            .iter()
            .find(|(name, _, _)| *name == dataset)
            .ok_or(ReceivableError::InvalidOption)?;
        if !(1..=500).contains(&limit) || after < 0 {
            return Err(ReceivableError::InvalidOption);
        }
        // All identifiers come exclusively from the private frozen catalog.
        let select = fields
            .iter()
            .map(|field| column(field))
            .collect::<Vec<_>>()
            .join(",");
        let records = client.query(
            &format!("SELECT json_build_array({select}) FROM {table} r WHERE r.tenant_id=$1 AND r.company_id=$2 AND r.id>$3 ORDER BY r.id LIMIT $4"),
            &[&scope.tenant(), &company_id, &after, &limit],
        ).await.map_err(Error::from)?;
        let rows: Vec<Vec<Value>> = records
            .iter()
            .map(|row| {
                let Value::Array(values) = row.get::<_, Value>(0) else {
                    unreachable!("json_build_array always returns an array")
                };
                values
            })
            .collect();
        let next = (rows.len() as i64 == limit)
            .then(|| {
                rows.last()
                    .and_then(|row| row.first())
                    .and_then(Value::as_i64)
            })
            .flatten();
        Ok(EvidencePage {
            fields: fields.to_vec(),
            rows,
            next,
        })
    }
}
