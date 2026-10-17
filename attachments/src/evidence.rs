//! The export contract (Bilimbi `Attachments.Evidence`): one ID-ordered page
//! of every attachment (with its file name, SHA-256 and retain-until), its
//! events, and the company's retention changes. An owned export contract, not
//! permission to consume the tables: the bytes themselves stay in Base
//! Artifacts and this index names each file, its hash and retain-until.
use crate::{operations::authorize, store::ISO, AttachmentError, Attachments, Result, VIEW};
use bilibi_database::Error;
use serde::Serialize;
use serde_json::Value;

pub(crate) const DATASETS: &[&str] = &["attachment", "attachment_event", "retention_change"];

const DEFAULT_LIMIT: i64 = 500;

/// One page: the explicit fields, the rows and the cursor of the next page
/// (the last row's id when the page was full).
#[derive(Clone, Debug, Serialize)]
pub struct EvidencePage {
    pub fields: Vec<&'static str>,
    pub rows: Vec<Value>,
    pub next: Option<i64>,
}

fn fields(dataset: &str) -> Option<&'static [&'static str]> {
    Some(match dataset {
        "attachment" => &[
            "id",
            "tenant_id",
            "company_id",
            "attachment_number",
            "source",
            "source_module",
            "document_id",
            "document_label",
            "document_date",
            "artifact_id",
            "file_name",
            "content_type",
            "byte_size",
            "sha256",
            "retention_years",
            "retain_until",
            "legal_hold",
            "status",
            "captured_by_user_id",
            "captured_by_impersonator_id",
            "captured_at",
        ],
        "attachment_event" => &[
            "id",
            "tenant_id",
            "company_id",
            "attachment_id",
            "event",
            "replacement_id",
            "retain_until",
            "reason",
            "actor_user_id",
            "actor_impersonator_id",
            "actor_system_principal",
            "occurred_at",
        ],
        "retention_change" => &[
            "id",
            "tenant_id",
            "company_id",
            "change_number",
            "years",
            "reason",
            "changed_by_user_id",
            "changed_by_impersonator_id",
            "changed_at",
        ],
        _ => return None,
    })
}

/// The attachment row keeps the retain-until it was captured with; the index
/// answers with the one in force (the latest extension), whether a legal hold
/// is on it (its latest hold event) and whether it still stands.
fn query(dataset: &str) -> String {
    let (select, from, readable) = match dataset {
        "attachment" => (
            format!(
                "r.id,r.tenant_id,r.company_id,r.attachment_number,r.source,r.source_module,r.document_id,r.document_label,\
                 to_char(r.document_date,'YYYY-MM-DD') AS document_date,r.artifact_id::text AS artifact_id,r.file_name,\
                 r.content_type,r.byte_size,r.sha256,r.retention_years,\
                 to_char(GREATEST(r.retain_until,(SELECT MAX(e.retain_until) FROM finance_attachment_events e WHERE e.attachment_id = r.id AND e.event = 'retention_extended')),'{ISO}') AS retain_until,\
                 COALESCE((SELECT e.event = 'hold_placed' FROM finance_attachment_events e WHERE e.attachment_id = r.id AND e.event IN ('hold_placed','hold_released') ORDER BY e.id DESC LIMIT 1),false) AS legal_hold,\
                 COALESCE((SELECT e.event FROM finance_attachment_events e WHERE e.attachment_id = r.id AND e.event IN ('replaced','removed')),'standing') AS status,\
                 r.captured_by_user_id,r.captured_by_impersonator_id,to_char(r.captured_at,'{ISO}') AS captured_at"
            ),
            "finance_attachments",
            " AND r.source = ANY($4)",
        ),
        "attachment_event" => (
            format!(
                "r.id,r.tenant_id,r.company_id,r.attachment_id,r.event,r.replacement_id,\
                 to_char(r.retain_until,'{ISO}') AS retain_until,r.reason,r.actor_user_id,r.actor_impersonator_id,\
                 r.actor_system_principal,to_char(r.occurred_at,'{ISO}') AS occurred_at"
            ),
            "finance_attachment_events",
            " AND r.attachment_id IN (SELECT a.id FROM finance_attachments a WHERE a.tenant_id = $1 AND a.company_id = $2 AND a.source = ANY($4))",
        ),
        _ => (
            format!(
                "r.id,r.tenant_id,r.company_id,r.change_number,r.years,r.reason,r.changed_by_user_id,\
                 r.changed_by_impersonator_id,to_char(r.changed_at,'{ISO}') AS changed_at"
            ),
            "finance_attachments_retention_changes",
            "",
        ),
    };
    format!(
        "SELECT COALESCE(json_agg(t ORDER BY t.id),'[]'::json) FROM (SELECT {select} FROM {from} r \
         WHERE r.tenant_id = $1 AND r.company_id = $2 AND r.id > $3{readable} ORDER BY r.id LIMIT $5) t"
    )
}

impl Attachments {
    /// One ID-ordered evidence page with explicit fields, rows and next cursor
    /// (`finance.attachments.view`): leaves out the attachments (and their
    /// events) of every source the caller cannot read, as the other reads do.
    /// `limit` is 1 to 500 (500 when absent) and `after` the last id of the
    /// previous page. Consume all pages in one repeatable-read transaction for
    /// a consistent export; individual pages are live reads.
    pub async fn evidence_page(
        &self,
        scope: &bilibi_tenancy::Scope,
        company_id: i64,
        dataset: &str,
        limit: Option<i64>,
        after: Option<i64>,
    ) -> Result<EvidencePage> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        let limit = limit.unwrap_or(DEFAULT_LIMIT);
        let after = after.unwrap_or(0);
        let fields = fields(dataset).ok_or(AttachmentError::InvalidOption)?;
        if !(1..=500).contains(&limit) || after < 0 {
            return Err(AttachmentError::InvalidOption);
        }
        let sources = self.readable_sources(&client, scope, company_id).await?;
        let row = client
            .query_one(
                &query(dataset),
                &[&scope.tenant(), &company_id, &after, &sources, &limit],
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
}
