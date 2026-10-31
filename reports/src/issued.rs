//! Issued report snapshots (Bilimbi `Bilimbi.Finance.Reports.Issued`): one
//! immutable row per issued report, with its content, chart, layout and
//! section membership as durable JSON. PostgreSQL stamps `issued_at` and
//! refuses update, delete and truncate. The evidence page exports the table
//! for the auditor's pack, as the pack's `issued-reports.csv`.
use crate::{ReportsError, Result};
use bilibi_database::Error;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

pub(crate) const DATASETS: &[&str] = &["issued"];

const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"";
const DEFAULT_LIMIT: i64 = 500;
const FIELDS: &[&str] = &[
    "id",
    "tenant_id",
    "company_id",
    "kind",
    "implementation_version",
    "through_entry_number",
    "content",
    "chart",
    "layout",
    "membership",
    "issued_by_user_id",
    "issued_by_impersonator_id",
    "issued_at",
];

/// The stored copy of an issued report, as the database holds it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct IssuedReport {
    pub id: i64,
    pub company_id: i64,
    pub kind: String,
    pub implementation_version: String,
    pub through_entry_number: i64,
    /// The report's figures, as its live read returned them.
    pub content: Value,
    /// The chart of accounts the figures were arranged by.
    pub chart: Value,
    /// `{"configured_sections": ...}`: the layout setting as it was read.
    pub layout: Value,
    /// For a statement, each covered account's section index
    /// (`account_sections`), the effective `sections` and `unmatched_codes`;
    /// `{}` for a trial balance.
    pub membership: Value,
    pub issued_by_user_id: i64,
    pub issued_by_impersonator_id: Option<i64>,
    /// ISO 8601 UTC, stamped by the database.
    pub issued_at: String,
}

pub(crate) struct NewIssued {
    pub tenant_id: i64,
    pub company_id: i64,
    pub kind: &'static str,
    pub implementation_version: &'static str,
    pub through_entry_number: i64,
    pub content: Value,
    pub chart: Value,
    pub layout: Value,
    pub membership: Value,
    pub issued_by_user_id: i64,
    pub issued_by_impersonator_id: Option<i64>,
}

fn columns() -> String {
    format!(
        "id,company_id,kind,implementation_version,through_entry_number,content,chart,layout,membership,\
         issued_by_user_id,issued_by_impersonator_id,to_char(issued_at,'{ISO}')"
    )
}

fn view(row: &Row) -> IssuedReport {
    IssuedReport {
        id: row.get(0),
        company_id: row.get(1),
        kind: row.get(2),
        implementation_version: row.get(3),
        through_entry_number: row.get(4),
        content: row.get(5),
        chart: row.get(6),
        layout: row.get(7),
        membership: row.get(8),
        issued_by_user_id: row.get(9),
        issued_by_impersonator_id: row.get(10),
        issued_at: row.get(11),
    }
}

pub(crate) async fn insert(
    client: &(impl GenericClient + Sync),
    new: &NewIssued,
) -> std::result::Result<IssuedReport, Error> {
    let row = client
        .query_one(
            &format!(
                "INSERT INTO finance_reports_issued(tenant_id,company_id,kind,implementation_version,\
                 through_entry_number,content,chart,layout,membership,issued_by_user_id,issued_by_impersonator_id,issued_at) \
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,now()) RETURNING {}",
                columns()
            ),
            &[
                &new.tenant_id,
                &new.company_id,
                &new.kind,
                &new.implementation_version,
                &new.through_entry_number,
                &new.content,
                &new.chart,
                &new.layout,
                &new.membership,
                &new.issued_by_user_id,
                &new.issued_by_impersonator_id,
            ],
        )
        .await?;
    Ok(view(&row))
}

/// What the audit records of an issue: the row without its JSON bodies, which
/// the immutable row itself keeps.
pub(crate) fn summary(issued: &IssuedReport) -> Value {
    json!({
        "company_id": issued.company_id,
        "kind": issued.kind,
        "implementation_version": issued.implementation_version,
        "through_entry_number": issued.through_entry_number,
        "issued_by_user_id": issued.issued_by_user_id,
        "issued_by_impersonator_id": issued.issued_by_impersonator_id,
        "issued_at": issued.issued_at,
    })
}

pub(crate) async fn find(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> std::result::Result<Option<IssuedReport>, Error> {
    Ok(client
        .query_opt(
            &format!(
                "SELECT {} FROM finance_reports_issued WHERE tenant_id=$1 AND company_id=$2 AND id=$3",
                columns()
            ),
            &[&tenant, &company_id, &id],
        )
        .await?
        .as_ref()
        .map(view))
}

/// One evidence page: the explicit fields, the rows and the cursor of the next
/// page (the last row's id when the page was full).
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Value>,
    pub next: Option<i64>,
}

pub(crate) async fn page(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    dataset: &str,
    limit: Option<i64>,
    after: Option<i64>,
) -> Result<EvidencePage> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    let after = after.unwrap_or(0);
    if !DATASETS.contains(&dataset) || !(1..=500).contains(&limit) || after < 0 {
        return Err(ReportsError::InvalidOption);
    }
    let row = client
        .query_one(
            &format!(
                "SELECT COALESCE(json_agg(t ORDER BY t.id),'[]'::json) FROM (SELECT id,tenant_id,company_id,kind,\
                 implementation_version,through_entry_number,content,chart,layout,membership,issued_by_user_id,\
                 issued_by_impersonator_id,to_char(issued_at,'{ISO}') AS issued_at FROM finance_reports_issued \
                 WHERE tenant_id=$1 AND company_id=$2 AND id>$3 ORDER BY id LIMIT $4) t"
            ),
            &[&tenant, &company_id, &after, &limit],
        )
        .await
        .map_err(Error::from)?;
    let rows = match row.get::<_, Value>(0) {
        Value::Array(rows) => rows,
        _ => Vec::new(),
    };
    let next = (rows.len() as i64 == limit)
        .then(|| rows.last().and_then(|row| row["id"].as_i64()))
        .flatten();
    Ok(EvidencePage {
        fields: FIELDS.to_vec(),
        rows,
        next,
    })
}
