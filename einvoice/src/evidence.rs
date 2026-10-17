//! Bilimbi.Finance.Einvoice.Evidence: frozen exports of this module's own
//! tables for the auditor pack. Never a credential: the company's client
//! secret, certificate and key live in Base Settings, not here.
use crate::{authorize, Einvoice, EinvoiceError, Result, VIEW};
use bilibi_database::Error;
use bilibi_tenancy::Scope;
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
        "code_override",
        "finance_einvoice_code_overrides",
        &[
            "id",
            "tenant_id",
            "company_id",
            "authority",
            "list",
            "code",
            "description",
            "active",
            "inserted_at",
            "updated_at",
        ],
    ),
    (
        "consolidated_source",
        "finance_einvoice_consolidated_sources",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "source_document_id",
            "line_number",
        ],
    ),
    (
        "document",
        "finance_einvoice_documents",
        &[
            "id",
            "tenant_id",
            "company_id",
            "authority",
            "environment",
            "document_type",
            "code_number",
            "issued_at",
            "currency_code",
            "exchange_rate",
            "supplier",
            "buyer",
            "details",
            "original_document_id",
            "line_extension_amount",
            "tax_exclusive_amount",
            "tax_amount",
            "tax_inclusive_amount",
            "payable_amount",
            "issued_by_user_id",
            "issued_by_impersonator_id",
            "recorded_at",
        ],
    ),
    (
        "line",
        "finance_einvoice_document_lines",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "line_number",
            "description",
            "classification_code",
            "tariff_code",
            "origin_country_code",
            "quantity",
            "unit_code",
            "unit_price",
            "subtotal",
            "discount_amount",
            "charge_amount",
            "line_extension_amount",
            "tax_type",
            "tax_rate",
            "tax_amount",
            "tax_exemption_reason",
        ],
    ),
    (
        "source_link",
        "finance_einvoice_source_links",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "source_module",
            "source_record",
            "ledger_entry_id",
        ],
    ),
    (
        "status_event",
        "finance_einvoice_status_events",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "sequence",
            "submission_id",
            "event",
            "authority_status",
            "submission_uid",
            "document_uuid",
            "long_id",
            "reason",
            "http_status",
            "response",
            "ledger_entry_id",
            "actor_type",
            "actor_user_id",
            "actor_impersonator_id",
            "system_principal",
            "recorded_at",
        ],
    ),
    (
        "submission",
        "finance_einvoice_submissions",
        &[
            "id",
            "tenant_id",
            "company_id",
            "document_id",
            "environment",
            "format",
            "document_version",
            "payload",
            "payload_sha256",
            "actor_type",
            "actor_user_id",
            "actor_impersonator_id",
            "system_principal",
            "recorded_at",
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

/// A field as the export writes it: instants in UTC, exact numbers as text,
/// and the submitted payload as the text it is.
fn column(field: &str) -> String {
    if field.ends_with("_at") {
        format!("to_char(r.{field},'{ISO}') AS {field}")
    } else if field == "payload" {
        "convert_from(r.payload,'UTF8') AS payload".into()
    } else if field.ends_with("_amount")
        || matches!(
            field,
            "exchange_rate" | "quantity" | "unit_price" | "subtotal" | "tax_rate"
        )
    {
        format!("r.{field}::text AS {field}")
    } else {
        format!("r.{field}")
    }
}

/// One page of `dataset` for the company: `InvalidOption` for an unknown
/// dataset, a limit outside 1 to 500, or a negative cursor.
async fn page(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    dataset: &str,
    limit: Option<i64>,
    after: Option<i64>,
) -> Result<EvidencePage> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let after = after.unwrap_or(0);
    let (_, table, fields) = DATASETS
        .iter()
        .find(|(name, _, _)| *name == dataset)
        .ok_or(EinvoiceError::InvalidOption)?;
    if !(1..=MAX_LIMIT).contains(&limit) || after < 0 {
        return Err(EinvoiceError::InvalidOption);
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

impl Einvoice {
    /// The evidence datasets, sorted: one per table of this module.
    pub fn evidence_datasets() -> Vec<&'static str> {
        DATASETS.iter().map(|(name, _, _)| *name).collect()
    }

    /// One ID-ordered page of a dataset (`finance.einvoice.view`): `limit` 1
    /// to 500 (default 500), rows after the id `after`. Read every page in
    /// one repeatable-read transaction for a consistent export; each page on
    /// its own is a live read.
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
        page(client, scope.tenant(), company_id, dataset, limit, after).await
    }
}
