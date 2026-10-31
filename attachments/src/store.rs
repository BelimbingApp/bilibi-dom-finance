//! The module's own SQL: attachments, their events and the company's
//! retention changes. Private to the module; the facade is the contract.
use crate::{Attachment, AttachmentEvent, RetentionChange, Status};
use bilibi_database::Result;
use jiff::{civil::Date, Timestamp};
use serde_json::{json, Value};
use tokio_postgres::{types::ToSql, GenericClient, Row};

pub(crate) const ISO: &str = "YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"";

const ATTACHMENT_COLUMNS: &str = "id,attachment_number,source,source_module,document_id,document_label,\
    to_char(document_date,'YYYY-MM-DD') AS document_date,artifact_id::text AS artifact_id,file_name,\
    content_type,byte_size,sha256,retention_years,\
    to_char(retain_until,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS retain_until,captured_by_user_id,\
    captured_by_impersonator_id,to_char(captured_at,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS captured_at";
const EVENT_COLUMNS: &str = "id,attachment_id,event,replacement_id,\
    to_char(retain_until,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS retain_until,reason,actor_user_id,\
    actor_impersonator_id,actor_system_principal,\
    to_char(occurred_at,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS occurred_at";
const CHANGE_COLUMNS: &str =
    "id,change_number,years,reason,changed_by_user_id,changed_by_impersonator_id,\
    to_char(changed_at,'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS changed_at";

pub(crate) fn timestamp(text: &str) -> Timestamp {
    text.parse().unwrap_or(Timestamp::UNIX_EPOCH)
}
pub(crate) fn date(text: &str) -> Date {
    text.parse().unwrap_or(Date::ZERO)
}
/// The text a `timestamp` column is written from (UTC).
pub(crate) fn iso(moment: Timestamp) -> String {
    moment.strftime("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()
}

/// One attachment before its events are attached.
struct Bare(Attachment);

fn attachment(row: &Row) -> Bare {
    Bare(Attachment {
        id: row.get("id"),
        attachment_number: row.get("attachment_number"),
        source: row.get("source"),
        source_module: row.get("source_module"),
        source_label: None,
        document_id: row.get("document_id"),
        document_label: row.get("document_label"),
        document_date: date(row.get::<_, &str>("document_date")),
        artifact_id: row.get("artifact_id"),
        file_name: row.get("file_name"),
        content_type: row.get("content_type"),
        byte_size: row.get("byte_size"),
        sha256: row.get("sha256"),
        retention_years: row.get("retention_years"),
        retain_until: timestamp(row.get::<_, &str>("retain_until")),
        captured_by_user_id: row.get("captured_by_user_id"),
        captured_by_impersonator_id: row.get("captured_by_impersonator_id"),
        captured_at: timestamp(row.get::<_, &str>("captured_at")),
        status: Status::Standing,
        on_hold: false,
        events: Vec::new(),
    })
}

fn event(row: &Row) -> (i64, AttachmentEvent) {
    (
        row.get("attachment_id"),
        AttachmentEvent {
            id: row.get("id"),
            event: row.get("event"),
            replacement_id: row.get("replacement_id"),
            retain_until: row.get::<_, Option<&str>>("retain_until").map(timestamp),
            reason: row.get("reason"),
            actor_user_id: row.get("actor_user_id"),
            actor_impersonator_id: row.get("actor_impersonator_id"),
            actor_system_principal: row.get("actor_system_principal"),
            occurred_at: timestamp(row.get::<_, &str>("occurred_at")),
        },
    )
}

/// Attaches each attachment's events (oldest first) and derives what they
/// decide: its standing, the retain-until in force and whether a hold stands.
async fn with_events(
    client: &(impl GenericClient + Sync),
    rows: Vec<Bare>,
) -> Result<Vec<Attachment>> {
    let ids: Vec<i64> = rows.iter().map(|Bare(a)| a.id).collect();
    let events = client
        .query(
            &format!("SELECT {EVENT_COLUMNS} FROM finance_attachment_events WHERE attachment_id = ANY($1) ORDER BY id"),
            &[&ids],
        )
        .await?;
    let events: Vec<_> = events.iter().map(event).collect();
    Ok(rows
        .into_iter()
        .map(|Bare(mut attachment)| {
            attachment.events = events
                .iter()
                .filter(|(owner, _)| *owner == attachment.id)
                .map(|(_, event)| event.clone())
                .collect();
            attachment.settle();
            attachment
        })
        .collect())
}

pub(crate) async fn fetch(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<Option<Attachment>> {
    let rows = client
        .query(
            &format!("SELECT {ATTACHMENT_COLUMNS} FROM finance_attachments WHERE tenant_id=$1 AND company_id=$2 AND id=$3"),
            &[&tenant, &company_id, &id],
        )
        .await?;
    Ok(with_events(client, rows.iter().map(attachment).collect())
        .await?
        .pop())
}

/// The standing attachments of one record, oldest first.
pub(crate) async fn standing_for_record(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    source: &str,
    document_id: i64,
) -> Result<Vec<Attachment>> {
    let rows = client
        .query(
            &format!("SELECT {ATTACHMENT_COLUMNS} FROM finance_attachments a WHERE tenant_id=$1 AND company_id=$2 AND source=$3 AND document_id=$4 AND {STANDING} ORDER BY id"),
            &[&tenant, &company_id, &source, &document_id],
        )
        .await?;
    with_events(client, rows.iter().map(attachment).collect()).await
}

const STANDING: &str = "NOT EXISTS (SELECT 1 FROM finance_attachment_events e WHERE e.attachment_id = a.id AND e.event IN ('replaced','removed'))";

/// The filters of the company's attachment list, already validated.
pub(crate) struct Filter<'a> {
    pub sources: &'a [String],
    pub source: Option<&'a str>,
    pub document_ids: Option<&'a [i64]>,
    pub status: &'a str,
    pub limit: i64,
    pub after: Option<i64>,
}

pub(crate) async fn list(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    filter: &Filter<'_>,
) -> Result<Vec<Attachment>> {
    let status = match filter.status {
        "standing" => format!("AND {STANDING}"),
        "replaced" | "removed" => "AND EXISTS (SELECT 1 FROM finance_attachment_events e WHERE e.attachment_id = a.id AND e.event = $8)".into(),
        _ => String::new(),
    };
    let sql = format!(
        "SELECT {ATTACHMENT_COLUMNS} FROM finance_attachments a WHERE tenant_id=$1 AND company_id=$2 \
         AND source = ANY($3) AND ($4::text IS NULL OR source=$4) AND ($5::bigint[] IS NULL OR document_id = ANY($5)) \
         AND ($6::bigint IS NULL OR attachment_number > $6) {status} ORDER BY attachment_number LIMIT $7"
    );
    let ids = filter.document_ids.map(<[i64]>::to_vec);
    let mut params: Vec<&(dyn ToSql + Sync)> = vec![
        &tenant,
        &company_id,
        &filter.sources,
        &filter.source,
        &ids,
        &filter.after,
        &filter.limit,
    ];
    if status.contains("$8") {
        params.push(&filter.status);
    }
    let rows = client.query(&sql, &params).await?;
    with_events(client, rows.iter().map(attachment).collect()).await
}

/// The distinct sources the company has attachments for, whatever their
/// standing.
pub(crate) async fn sources(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<String>> {
    Ok(client
        .query(
            "SELECT DISTINCT source FROM finance_attachments WHERE tenant_id=$1 AND company_id=$2 ORDER BY source",
            &[&tenant, &company_id],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

/// What the capture stored, to be linked to a record.
pub(crate) struct NewAttachment<'a> {
    pub source: &'a str,
    pub source_module: &'a str,
    pub document_id: i64,
    pub document_label: &'a str,
    pub document_date: Date,
    pub artifact_id: &'a str,
    pub file_name: &'a str,
    pub content_type: &'a str,
    pub byte_size: i64,
    pub sha256: &'a str,
    pub retention_years: i32,
    pub retain_until: Timestamp,
    pub captured_by: i64,
    pub impersonator: Option<i64>,
}

/// Inserts an attachment. The BEFORE INSERT trigger numbers and stamps it
/// before NOT NULL is checked, so the number and time are sent as NULL.
pub(crate) async fn insert(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    new: &NewAttachment<'_>,
) -> Result<i64> {
    let row = client
        .query_one(
            "INSERT INTO finance_attachments(tenant_id,company_id,attachment_number,source,source_module,document_id,\
             document_label,document_date,artifact_id,file_name,content_type,byte_size,sha256,retention_years,\
             retain_until,captured_by_user_id,captured_by_impersonator_id,captured_at) \
             VALUES($1,$2,NULL,$3,$4,$5,$6,$7::text::date,$8::text::uuid,$9,$10,$11,$12,$13,\
             ($14::text)::timestamptz AT TIME ZONE 'UTC',$15,$16,NULL) RETURNING id",
            &[
                &tenant,
                &company_id,
                &new.source,
                &new.source_module,
                &new.document_id,
                &new.document_label,
                &new.document_date.to_string(),
                &new.artifact_id,
                &new.file_name,
                &new.content_type,
                &new.byte_size,
                &new.sha256,
                &new.retention_years,
                &iso(new.retain_until),
                &new.captured_by,
                &new.impersonator,
            ],
        )
        .await?;
    Ok(row.get(0))
}

pub(crate) struct NewEvent<'a> {
    pub attachment_id: i64,
    pub event: &'a str,
    pub replacement_id: Option<i64>,
    pub retain_until: Option<Timestamp>,
    pub reason: Option<&'a str>,
    pub user: i64,
    pub impersonator: Option<i64>,
    pub principal: Option<&'a str>,
}

pub(crate) async fn insert_event(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    new: &NewEvent<'_>,
) -> Result<(i64, Value)> {
    let until = new.retain_until.map(iso);
    let row = client
        .query_one(
            "INSERT INTO finance_attachment_events(tenant_id,company_id,attachment_id,event,replacement_id,retain_until,\
             reason,actor_user_id,actor_impersonator_id,actor_system_principal,occurred_at) \
             VALUES($1,$2,$3,$4,$5,($6::text)::timestamptz AT TIME ZONE 'UTC',$7,$8,$9,$10,NULL) RETURNING id",
            &[
                &tenant,
                &company_id,
                &new.attachment_id,
                &new.event,
                &new.replacement_id,
                &until,
                &new.reason,
                &new.user,
                &new.impersonator,
                &new.principal,
            ],
        )
        .await?;
    Ok((
        row.get(0),
        json!({
            "attachment_id": new.attachment_id, "event": new.event, "replacement_id": new.replacement_id,
            "retain_until": until, "reason": new.reason, "actor_user_id": new.user,
            "actor_impersonator_id": new.impersonator, "actor_system_principal": new.principal,
        }),
    ))
}

/// Locks an attachment's row so two writers appending its retention events
/// serialize.
pub(crate) async fn lock(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    id: i64,
) -> Result<bool> {
    Ok(client
        .query_opt(
            "SELECT id FROM finance_attachments WHERE tenant_id=$1 AND company_id=$2 AND id=$3 FOR UPDATE",
            &[&tenant, &company_id, &id],
        )
        .await?
        .is_some())
}

fn change(row: &Row) -> RetentionChange {
    RetentionChange {
        id: row.get("id"),
        change_number: row.get("change_number"),
        years: row.get("years"),
        reason: row.get("reason"),
        changed_by_user_id: row.get("changed_by_user_id"),
        changed_by_impersonator_id: row.get("changed_by_impersonator_id"),
        changed_at: timestamp(row.get::<_, &str>("changed_at")),
    }
}

pub(crate) async fn latest_change(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Option<RetentionChange>> {
    Ok(client
        .query_opt(
            &format!("SELECT {CHANGE_COLUMNS} FROM finance_attachments_retention_changes WHERE tenant_id=$1 AND company_id=$2 ORDER BY change_number DESC LIMIT 1"),
            &[&tenant, &company_id],
        )
        .await?
        .map(|row| change(&row)))
}

pub(crate) async fn changes(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
) -> Result<Vec<RetentionChange>> {
    Ok(client
        .query(
            &format!("SELECT {CHANGE_COLUMNS} FROM finance_attachments_retention_changes WHERE tenant_id=$1 AND company_id=$2 ORDER BY change_number"),
            &[&tenant, &company_id],
        )
        .await?
        .iter()
        .map(change)
        .collect())
}

pub(crate) async fn insert_change(
    client: &(impl GenericClient + Sync),
    tenant: i64,
    company_id: i64,
    years: i32,
    reason: &str,
    user: i64,
    impersonator: Option<i64>,
) -> Result<RetentionChange> {
    let row = client
        .query_one(
            &format!("INSERT INTO finance_attachments_retention_changes(tenant_id,company_id,change_number,years,reason,changed_by_user_id,changed_by_impersonator_id,changed_at) \
             VALUES($1,$2,NULL,$3,$4,$5,$6,NULL) RETURNING {CHANGE_COLUMNS}"),
            &[&tenant, &company_id, &years, &reason, &user, &impersonator],
        )
        .await?;
    Ok(change(&row))
}
