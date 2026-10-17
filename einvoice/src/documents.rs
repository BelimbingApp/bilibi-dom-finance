//! Reads over a company's recorded documents, lines, submissions and status
//! events, the view the facade returns, and the one writer of status events
//! (Bilimbi `Bilimbi.Finance.Einvoice.Documents`). Every query names the
//! tenant and company; callers authorize first. Every write is a `mutate` of
//! the caller's audited transaction, returning each row as a `Change`.
use crate::authority::{Authority, Payload};
use crate::document::{Details, Document, DocumentLine, PartySnapshot};
use crate::draft::ValidDraft;
use bilibi_database::{Change, Error, MutationTransaction, Result};
use bilibi_tenancy::Scope;
use jiff::{SignedDuration, Timestamp};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_postgres::{GenericClient, Row};

const STAMP: &str = "'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"'";

/// Events that move a document's status; any other event records an answer,
/// a posting or a notice without changing it. `consolidated` and `released`
/// move a source in and out of a consolidated document.
pub(crate) const STATUS_EVENTS: [&str; 9] = [
    "issued",
    "submitted",
    "refused",
    "valid",
    "invalid",
    "cancelled",
    "rejected",
    "consolidated",
    "released",
];
/// The events that end an attempt: the authority answered, whatever it said.
const ANSWERS: [&str; 4] = ["submitted", "refused", "duplicate", "rate_limited"];
/// The events after which a consolidated document releases its sources.
pub(crate) const RELEASING: [&str; 3] = ["refused", "invalid", "cancelled"];
/// A status event keeps at most this much of a reason.
const REASON_LENGTH: usize = 300;

/// One status event, in recording order.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StatusEvent {
    /// 1, 2, 3, ... per document, numbered by the database.
    pub sequence: i32,
    pub event: String,
    pub submission_id: Option<i64>,
    pub authority_status: Option<String>,
    pub submission_uid: Option<String>,
    pub document_uuid: Option<String>,
    pub long_id: Option<String>,
    pub reason: Option<String>,
    pub http_status: Option<i32>,
    /// The authority's answer as it came.
    pub response: Option<Value>,
    pub ledger_entry_id: Option<i64>,
    pub actor_type: String,
    pub actor_user_id: Option<i64>,
    pub actor_impersonator_id: Option<i64>,
    pub system_principal: Option<String>,
    pub recorded_at: Timestamp,
}

/// What an operator should look at.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Attention {
    /// Not submitted within the authority's outage allowance of the authority
    /// becoming unavailable.
    Overdue,
    /// The buyer asked the authority to reject it.
    RejectionRequested,
}

/// A recorded document as the facade returns it: the document with its lines,
/// its status events, its current `status` (the latest status-moving event)
/// and what the events say about it.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentView {
    pub document: Document,
    /// The authority it is cleared with.
    pub authority: String,
    pub status: String,
    pub issued_by_user_id: i64,
    pub issued_by_impersonator_id: Option<i64>,
    pub recorded_at: Timestamp,
    /// The counterparty's name: the buyer, or the supplier of a self-billed
    /// document.
    pub counterparty_name: Option<String>,
    pub self_billed: bool,
    pub consolidated: bool,
    /// The latest accepted submission's UID.
    pub submission_uid: Option<String>,
    pub document_uuid: Option<String>,
    pub long_id: Option<String>,
    pub validated_at: Option<Timestamp>,
    /// The authority's public link for a validated document.
    pub validation_url: Option<String>,
    /// When the supplier's cancellation window ends.
    pub cancellable_until: Option<Timestamp>,
    pub attention: Vec<Attention>,
    /// Whether a submit job for the document is waiting or running.
    pub submission_queued: bool,
    pub events: Vec<StatusEvent>,
}

/// A recorded document and its events, before the view adds what needs
/// settings or the queue.
pub(crate) struct Recorded {
    pub document: Document,
    pub authority: String,
    pub issued_by_user_id: i64,
    pub issued_by_impersonator_id: Option<i64>,
    pub recorded_at: Timestamp,
    pub events: Vec<StatusEvent>,
}

impl Recorded {
    /// The current status: the latest status-moving event.
    pub fn status(&self) -> &str {
        self.events
            .iter()
            .rev()
            .find(|event| STATUS_EVENTS.contains(&event.event.as_str()))
            .map_or("issued", |event| event.event.as_str())
    }

    fn latest(&self, name: &str) -> Option<&StatusEvent> {
        self.events.iter().rev().find(|event| event.event == name)
    }

    /// The events since the latest status-moving one, in order.
    pub fn since_status(&self) -> &[StatusEvent] {
        let start = self
            .events
            .iter()
            .rposition(|event| STATUS_EVENTS.contains(&event.event.as_str()))
            .map_or(0, |index| index + 1);
        &self.events[start..]
    }

    pub fn submission_uid(&self) -> Option<&str> {
        self.latest("submitted")?.submission_uid.as_deref()
    }

    pub fn document_uuid(&self) -> Option<&str> {
        self.events
            .iter()
            .rev()
            .find_map(|event| event.document_uuid.as_deref())
    }

    fn long_id(&self) -> Option<&str> {
        self.events
            .iter()
            .rev()
            .find_map(|event| event.long_id.as_deref())
    }

    /// When the authority validated it: its own instant, or when the `valid`
    /// event was recorded.
    pub fn validated_at(&self) -> Option<Timestamp> {
        let valid = self.latest("valid")?;
        Some(
            valid
                .response
                .as_ref()
                .and_then(|response| response.get("dateTimeValidated"))
                .and_then(Value::as_str)
                .and_then(|text| text.parse().ok())
                .unwrap_or(valid.recorded_at),
        )
    }

    pub fn cancellable_until(&self, authority: &impl Authority) -> Option<Timestamp> {
        let hours = i64::from(authority.cancellation_window_hours());
        self.validated_at()
            .map(|at| at + SignedDuration::from_hours(hours))
    }

    /// Every document UUID the document's events recorded.
    pub fn known_uuids(&self) -> Vec<String> {
        let mut uuids: Vec<String> = Vec::new();
        for uuid in self.events.iter().filter_map(|e| e.document_uuid.clone()) {
            if !uuids.contains(&uuid) {
                uuids.push(uuid);
            }
        }
        uuids
    }

    pub fn consolidated(&self) -> bool {
        self.document.details.consolidation_month.is_some()
    }

    /// The view, given the portal's base URL for the validation link and
    /// whether a submit job is waiting.
    pub fn view(
        self,
        authority: &impl Authority,
        portal_url: &str,
        submission_queued: bool,
    ) -> DocumentView {
        let status = self.status().to_owned();
        let since_status = self.since_status();
        let mut attention = Vec::new();
        if since_status.iter().any(|event| event.event == "overdue") {
            attention.push(Attention::Overdue);
        }
        if status == "rejected" {
            attention.push(Attention::RejectionRequested);
        }
        let self_billed = authority
            .self_billed_document_types()
            .contains(&self.document.document_type.as_str());
        let counterparty = if self_billed {
            &self.document.supplier
        } else {
            &self.document.buyer
        };
        let document_uuid = self.document_uuid().map(str::to_owned);
        let long_id = self.long_id().map(str::to_owned);
        DocumentView {
            counterparty_name: counterparty.name.clone(),
            self_billed,
            consolidated: self.consolidated(),
            submission_uid: self.submission_uid().map(str::to_owned),
            validation_url: document_uuid
                .as_deref()
                .zip(long_id.as_deref())
                .map(|(uuid, long_id)| authority.validation_url(portal_url, uuid, long_id)),
            document_uuid,
            long_id,
            validated_at: self.validated_at(),
            cancellable_until: self.cancellable_until(authority),
            attention,
            submission_queued,
            status,
            authority: self.authority,
            issued_by_user_id: self.issued_by_user_id,
            issued_by_impersonator_id: self.issued_by_impersonator_id,
            recorded_at: self.recorded_at,
            events: self.events,
            document: self.document,
        }
    }
}

fn stamp(row: &Row, column: &str) -> Result<Timestamp> {
    row.get::<_, String>(column)
        .parse()
        .map_err(|_| Error::Internal)
}

fn json_into<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|_| Error::Internal)
}

/// A document with its ordered lines and events, or `None`.
pub(crate) async fn fetch(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    document_id: i64,
) -> Result<Option<Recorded>> {
    let sql = format!(
        "SELECT id, authority, environment, document_type, code_number, \
         to_char(issued_at, {STAMP}) AS issued_at, currency_code, exchange_rate, supplier, buyer, \
         details, original_document_id, line_extension_amount, tax_exclusive_amount, tax_amount, \
         tax_inclusive_amount, payable_amount, issued_by_user_id, issued_by_impersonator_id, \
         to_char(recorded_at, {STAMP}) AS recorded_at \
         FROM finance_einvoice_documents WHERE tenant_id = $1 AND company_id = $2 AND id = $3"
    );
    let Some(row) = client
        .query_opt(&sql, &[&tenant, &company_id, &document_id])
        .await?
    else {
        return Ok(None);
    };
    let lines = client
        .query(
            "SELECT line_number, description, classification_code, tariff_code, \
             origin_country_code, quantity, unit_code, unit_price, subtotal, discount_amount, \
             charge_amount, line_extension_amount, tax_type, tax_rate, tax_amount, \
             tax_exemption_reason FROM finance_einvoice_document_lines \
             WHERE tenant_id = $1 AND company_id = $2 AND document_id = $3 ORDER BY line_number",
            &[&tenant, &company_id, &document_id],
        )
        .await?
        .iter()
        .map(|line| DocumentLine {
            line_number: line.get("line_number"),
            description: line.get("description"),
            classification_code: line.get("classification_code"),
            tariff_code: line.get("tariff_code"),
            origin_country_code: line.get("origin_country_code"),
            quantity: line.get("quantity"),
            unit_code: line.get("unit_code"),
            unit_price: line.get("unit_price"),
            subtotal: line.get("subtotal"),
            discount_amount: line.get("discount_amount"),
            charge_amount: line.get("charge_amount"),
            line_extension_amount: line.get("line_extension_amount"),
            tax_type: line.get("tax_type"),
            tax_rate: line.get("tax_rate"),
            tax_amount: line.get("tax_amount"),
            tax_exemption_reason: line.get("tax_exemption_reason"),
        })
        .collect();
    let document = Document {
        id: row.get("id"),
        environment: row.get("environment"),
        document_type: row.get("document_type"),
        code_number: row.get("code_number"),
        issued_at: stamp(&row, "issued_at")?,
        currency_code: row.get("currency_code"),
        exchange_rate: row.get("exchange_rate"),
        supplier: json_into::<PartySnapshot>(row.get("supplier"))?,
        buyer: json_into::<PartySnapshot>(row.get("buyer"))?,
        details: json_into::<Details>(row.get("details"))?,
        original_document_id: row.get("original_document_id"),
        line_extension_amount: row.get("line_extension_amount"),
        tax_exclusive_amount: row.get("tax_exclusive_amount"),
        tax_amount: row.get("tax_amount"),
        tax_inclusive_amount: row.get("tax_inclusive_amount"),
        payable_amount: row.get("payable_amount"),
        lines,
    };
    Ok(Some(Recorded {
        authority: row.get("authority"),
        issued_by_user_id: row.get("issued_by_user_id"),
        issued_by_impersonator_id: row.get("issued_by_impersonator_id"),
        recorded_at: stamp(&row, "recorded_at")?,
        events: events(client, tenant, company_id, document_id).await?,
        document,
    }))
}

/// A document's events in recording order.
pub(crate) async fn events(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    document_id: i64,
) -> Result<Vec<StatusEvent>> {
    let sql = format!(
        "SELECT sequence, event, submission_id, authority_status, submission_uid, document_uuid, \
         long_id, reason, http_status, response, ledger_entry_id, actor_type, actor_user_id, \
         actor_impersonator_id, system_principal, to_char(recorded_at, {STAMP}) AS recorded_at \
         FROM finance_einvoice_status_events \
         WHERE tenant_id = $1 AND company_id = $2 AND document_id = $3 ORDER BY sequence"
    );
    client
        .query(&sql, &[&tenant, &company_id, &document_id])
        .await?
        .iter()
        .map(|row| {
            Ok(StatusEvent {
                sequence: row.get("sequence"),
                event: row.get("event"),
                submission_id: row.get("submission_id"),
                authority_status: row.get("authority_status"),
                submission_uid: row.get("submission_uid"),
                document_uuid: row.get("document_uuid"),
                long_id: row.get("long_id"),
                reason: row.get("reason"),
                http_status: row.get("http_status"),
                response: row.get("response"),
                ledger_entry_id: row.get("ledger_entry_id"),
                actor_type: row.get("actor_type"),
                actor_user_id: row.get("actor_user_id"),
                actor_impersonator_id: row.get("actor_impersonator_id"),
                system_principal: row.get("system_principal"),
                recorded_at: stamp(row, "recorded_at")?,
            })
        })
        .collect()
}

/// The current status of each document, by ID.
pub(crate) async fn statuses(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    document_ids: &[i64],
) -> Result<Vec<(i64, String)>> {
    if document_ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(client
        .query(
            "SELECT DISTINCT ON (document_id) document_id, event \
             FROM finance_einvoice_status_events \
             WHERE tenant_id = $1 AND company_id = $2 AND document_id = ANY($3) \
               AND event = ANY($4) \
             ORDER BY document_id, sequence DESC",
            &[&tenant, &company_id, &document_ids, &&STATUS_EVENTS[..]],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect())
}

/// The document's current status, or `None` for a document the company does
/// not have.
pub(crate) async fn status(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    document_id: i64,
) -> Result<Option<String>> {
    Ok(statuses(client, tenant, company_id, &[document_id])
        .await?
        .pop()
        .map(|(_, status)| status))
}

/// Whether the company already issued `code_number` in the environment.
pub(crate) async fn code_number_taken(
    client: &impl GenericClient,
    company_id: i64,
    environment: &str,
    code_number: &str,
) -> Result<bool> {
    Ok(client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM finance_einvoice_documents \
             WHERE company_id = $1 AND environment = $2 AND code_number = $3)",
            &[&company_id, &environment, &code_number],
        )
        .await?
        .get(0))
}

/// The notes of the company that name the document as their original.
pub(crate) async fn notes_of(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    document_id: i64,
) -> Result<Vec<i64>> {
    Ok(client
        .query(
            "SELECT id FROM finance_einvoice_documents \
             WHERE tenant_id = $1 AND company_id = $2 AND original_document_id = $3",
            &[&tenant, &company_id, &document_id],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect())
}

/// Whether another document that stands links one of this document's
/// sources (the documents it covers do not count). The trigger on submissions
/// refuses to store one in that case, so this is asked before storing, under
/// the company lock.
pub(crate) async fn source_taken(client: &impl GenericClient, document_id: i64) -> Result<bool> {
    Ok(client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM finance_einvoice_source_links l \
             WHERE l.document_id = $1 \
               AND finance_einvoice_standing_source_link(l, $1) IS NOT NULL)",
            &[&document_id],
        )
        .await?
        .get(0))
}

/// One stored submission attempt.
#[derive(Clone, Debug)]
pub(crate) struct Attempt {
    pub id: i64,
    pub recorded_at: Timestamp,
}

/// The submissions stored since the document last became submittable whose
/// final answer is missing (a crash, an outage, or a search that could not
/// answer): any of them may have been accepted. Newest first.
pub(crate) async fn unresolved_attempts(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    recorded: &Recorded,
) -> Result<Vec<Attempt>> {
    let Some(last_status) = recorded
        .events
        .iter()
        .rev()
        .find(|event| STATUS_EVENTS.contains(&event.event.as_str()))
    else {
        return Ok(Vec::new());
    };
    let settled: Vec<i64> = recorded
        .since_status()
        .iter()
        .filter(|event| ANSWERS.contains(&event.event.as_str()))
        .filter_map(|event| event.submission_id)
        .collect();
    let sql = format!(
        "SELECT id, to_char(recorded_at, {STAMP}) AS recorded_at \
         FROM finance_einvoice_submissions \
         WHERE tenant_id = $1 AND company_id = $2 AND document_id = $3 \
           AND recorded_at > (SELECT recorded_at FROM finance_einvoice_status_events \
                              WHERE document_id = $3 AND sequence = $4) \
         ORDER BY id DESC"
    );
    client
        .query(
            &sql,
            &[
                &tenant,
                &company_id,
                &recorded.document.id,
                &last_status.sequence,
            ],
        )
        .await?
        .iter()
        .filter(|row| !settled.contains(&row.get::<_, i64>("id")))
        .map(|row| {
            Ok(Attempt {
                id: row.get("id"),
                recorded_at: stamp(row, "recorded_at")?,
            })
        })
        .collect()
}

/// Who a recorded row names: the scope's user and any impersonator.
fn actor(scope: &Scope) -> (i64, Option<i64>) {
    (
        scope.user(),
        scope.impersonator().map(|operator| operator.user()),
    )
}

/// A status event to record.
#[derive(Clone, Debug, Default)]
pub(crate) struct NewEvent {
    pub event: &'static str,
    pub submission_id: Option<i64>,
    pub authority_status: Option<&'static str>,
    pub submission_uid: Option<String>,
    pub document_uuid: Option<String>,
    pub long_id: Option<String>,
    pub reason: Option<String>,
    pub http_status: Option<u16>,
    pub response: Option<Value>,
}

impl NewEvent {
    pub fn new(event: &'static str) -> Self {
        Self {
            event,
            ..Self::default()
        }
    }
}

/// Records a status event for the document as the scope's user. A
/// consolidated document the authority refused, found invalid or cancelled
/// releases the documents it covered, in the same transaction.
pub(crate) async fn record_event(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    document_id: i64,
    event: NewEvent,
) -> Result<()> {
    let releasing = RELEASING.contains(&event.event);
    insert_event(tx, scope, company_id, document_id, event).await?;
    if releasing {
        release_sources(tx, scope, company_id, document_id).await?;
    }
    Ok(())
}

async fn insert_event(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    document_id: i64,
    event: NewEvent,
) -> Result<()> {
    let tenant = scope.tenant();
    let (user, impersonator) = actor(scope);
    let reason = event
        .reason
        .map(|reason| reason.chars().take(REASON_LENGTH).collect::<String>());
    let http_status = event.http_status.map(i32::from);
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    "INSERT INTO finance_einvoice_status_events (tenant_id, company_id, \
                     document_id, submission_id, event, authority_status, submission_uid, \
                     document_uuid, long_id, reason, http_status, response, actor_type, \
                     actor_user_id, actor_impersonator_id) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, 'user', $13, $14) \
                     RETURNING id, sequence",
                    &[
                        &tenant,
                        &company_id,
                        &document_id,
                        &event.submission_id,
                        &event.event,
                        &event.authority_status,
                        &event.submission_uid,
                        &event.document_uuid,
                        &event.long_id,
                        &reason,
                        &http_status,
                        &event.response,
                        &user,
                        &impersonator,
                    ],
                )
                .await?;
            let change = Change {
                kind: "finance.einvoice.status_event".into(),
                id: row.get("id"),
                old: Value::Null,
                // The authority's answer stays in the row; the audit names
                // what was recorded.
                new: json!({
                    "document_id": document_id,
                    "sequence": row.get::<_, i32>("sequence"),
                    "event": event.event,
                    "submission_id": event.submission_id,
                    "authority_status": event.authority_status,
                    "submission_uid": event.submission_uid,
                    "document_uuid": event.document_uuid,
                    "long_id": event.long_id,
                    "reason": reason,
                    "http_status": http_status,
                }),
            };
            Ok(((), vec![change]))
        })
    })
    .await
}

/// Records `released` on each source a consolidated document holds, once the
/// authority refused it, found it invalid or cancelled it. A source already
/// released, or never consolidated, is left alone.
async fn release_sources(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    document_id: i64,
) -> Result<()> {
    let tenant = scope.tenant();
    let sources: Vec<i64> = tx
        .sql()
        .query(
            "SELECT DISTINCT source_document_id FROM finance_einvoice_consolidated_sources \
             WHERE tenant_id = $1 AND company_id = $2 AND document_id = $3",
            &[&tenant, &company_id, &document_id],
        )
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    if sources.is_empty() {
        return Ok(());
    }
    let row = tx
        .sql()
        .query_one(
            "SELECT code_number, (SELECT event FROM finance_einvoice_status_events \
             WHERE document_id = $1 ORDER BY sequence DESC LIMIT 1) \
             FROM finance_einvoice_documents WHERE id = $1",
            &[&document_id],
        )
        .await?;
    let (code_number, event): (String, String) = (row.get(0), row.get(1));
    let mut statuses = statuses(tx.sql(), tenant, company_id, &sources).await?;
    statuses.sort();
    for (source_id, status) in statuses {
        if status == "consolidated" {
            let mut released = NewEvent::new("released");
            released.reason = Some(format!("{code_number} is {event}"));
            insert_event(tx, scope, company_id, source_id, released).await?;
        }
    }
    Ok(())
}

/// Stores a submission's exact bytes before the authority sees them, with
/// their hex SHA-256; the database checks the hash against the bytes.
pub(crate) async fn store_submission(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    document: &Document,
    payload: &Payload,
    sha256: String,
) -> Result<Attempt> {
    let tenant = scope.tenant();
    let (format, version, bytes) = (
        payload.format,
        payload.version.clone(),
        payload.bytes.clone(),
    );
    let (user, impersonator) = actor(scope);
    let (document_id, environment) = (document.id, document.environment.clone());
    tx.mutate(move |sql| {
        Box::pin(async move {
            let row = sql
                .query_one(
                    &format!(
                        "INSERT INTO finance_einvoice_submissions (tenant_id, company_id, \
                         document_id, environment, format, document_version, payload, \
                         payload_sha256, actor_type, actor_user_id, actor_impersonator_id) \
                         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'user', $9, $10) \
                         RETURNING id, to_char(recorded_at, {STAMP}) AS recorded_at"
                    ),
                    &[
                        &tenant,
                        &company_id,
                        &document_id,
                        &environment,
                        &format,
                        &version,
                        &bytes,
                        &sha256,
                        &user,
                        &impersonator,
                    ],
                )
                .await?;
            let attempt = Attempt {
                id: row.get("id"),
                recorded_at: stamp(&row, "recorded_at")?,
            };
            let change = Change {
                kind: "finance.einvoice.submission".into(),
                id: attempt.id,
                old: Value::Null,
                new: json!({
                    "document_id": document_id, "environment": environment, "format": format,
                    "document_version": version, "payload_sha256": sha256,
                }),
            };
            Ok((attempt, vec![change]))
        })
    })
    .await
}

/// Records an issued document, its lines and its `issued` event: the
/// company's profile is the supplier, or the buyer of a self-billed document.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn insert_document(
    tx: &mut MutationTransaction<'_>,
    scope: &Scope,
    company_id: i64,
    authority: &'static str,
    environment: String,
    draft: ValidDraft,
    supplier: PartySnapshot,
    buyer: PartySnapshot,
    details: Details,
) -> Result<i64> {
    let tenant = scope.tenant();
    let (user, impersonator) = actor(scope);
    let document_id = tx
        .mutate(move |sql| {
            Box::pin(async move {
                let total = draft.line_extension_amount + draft.tax_amount;
                let issued_at = draft.issued_at.to_string();
                let supplier = serde_json::to_value(&supplier).map_err(|_| Error::Internal)?;
                let buyer = serde_json::to_value(&buyer).map_err(|_| Error::Internal)?;
                let details = serde_json::to_value(&details).map_err(|_| Error::Internal)?;
                let row = sql
                    .query_one(
                        "INSERT INTO finance_einvoice_documents (tenant_id, company_id, \
                         authority, environment, document_type, code_number, issued_at, \
                         currency_code, exchange_rate, supplier, buyer, details, \
                         original_document_id, line_extension_amount, tax_exclusive_amount, \
                         tax_amount, tax_inclusive_amount, payable_amount, issued_by_user_id, \
                         issued_by_impersonator_id) \
                         VALUES ($1, $2, $3, $4, $5, $6, \
                         ($7::text::timestamptz AT TIME ZONE 'UTC')::timestamp(0), \
                         $8, $9, $10, $11, $12, $13, $14, $14, $15, $16, $16, $17, $18) \
                         RETURNING id",
                        &[
                            &tenant,
                            &company_id,
                            &authority,
                            &environment,
                            &draft.document_type,
                            &draft.code_number,
                            &issued_at,
                            &draft.currency_code,
                            &draft.exchange_rate,
                            &supplier,
                            &buyer,
                            &details,
                            &draft.original_document_id,
                            &draft.line_extension_amount,
                            &draft.tax_amount,
                            &total,
                            &user,
                            &impersonator,
                        ],
                    )
                    .await?;
                let id: i64 = row.get(0);
                let mut changes = vec![Change {
                    kind: "finance.einvoice.document".into(),
                    id,
                    old: Value::Null,
                    new: json!({
                        "authority": authority, "environment": environment,
                        "document_type": draft.document_type, "code_number": draft.code_number,
                        "issued_at": issued_at, "currency_code": draft.currency_code,
                        "original_document_id": draft.original_document_id,
                        "line_extension_amount": draft.line_extension_amount.to_string(),
                        "tax_amount": draft.tax_amount.to_string(),
                        "payable_amount": total.to_string(),
                    }),
                }];
                for (number, line) in (1_i32..).zip(&draft.lines) {
                    let row = sql
                        .query_one(
                            "INSERT INTO finance_einvoice_document_lines (tenant_id, company_id, \
                             document_id, line_number, description, classification_code, \
                             tariff_code, origin_country_code, quantity, unit_code, unit_price, \
                             subtotal, discount_amount, charge_amount, line_extension_amount, \
                             tax_type, tax_rate, tax_amount, tax_exemption_reason) \
                             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                             $14, $15, $16, $17, $18, $19) RETURNING id",
                            &[
                                &tenant,
                                &company_id,
                                &id,
                                &number,
                                &line.description,
                                &line.classification_code,
                                &line.tariff_code,
                                &line.origin_country_code,
                                &line.quantity,
                                &line.unit_code,
                                &line.unit_price,
                                &line.subtotal,
                                &line.discount_amount,
                                &line.charge_amount,
                                &line.line_extension_amount,
                                &line.tax_type,
                                &line.tax_rate,
                                &line.tax_amount,
                                &line.tax_exemption_reason,
                            ],
                        )
                        .await?;
                    changes.push(Change {
                        kind: "finance.einvoice.document_line".into(),
                        id: row.get(0),
                        old: Value::Null,
                        new: json!({
                            "document_id": id, "line_number": number,
                            "line_extension_amount": line.line_extension_amount.to_string(),
                            "tax_amount": line.tax_amount.to_string(),
                        }),
                    });
                }
                Ok((id, changes))
            })
        })
        .await?;
    insert_event(tx, scope, company_id, document_id, NewEvent::new("issued")).await?;
    Ok(document_id)
}
