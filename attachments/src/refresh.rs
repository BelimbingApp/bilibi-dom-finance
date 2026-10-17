//! The retention refresh (Bilimbi `RetentionRefreshWorker`): extends the
//! retention of a record's source documents once the record is final, as the
//! system principal `finance-attachments-retention-refresh`.
//!
//! Queued by [`Attachments::refresh_retention`] inside the transaction that
//! finalises the record, after the caller's own scope has confirmed that the
//! record is readable and final and has computed the retain-until its final
//! date gives. The arguments are the tenant, the company, the source id, the
//! record's id, that retain-until and the person who finalised the record: no
//! file name, label or secret. The job extends each standing attachment of the
//! record that is kept for less, through Base Artifacts (which only ever
//! lengthens), and records `retention_extended` as the principal. A failure
//! retries under the queue's backoff; the job can be queued again.
//!
//! The principal must hold `finance.attachments.retention.manage` in the
//! tenant (`Grants::set_system`); without it the job cancels as
//! `actor_refused` and a later refresh queues it again.
//!
//! TODO(platform): Base Artifacts accepts only a user's sealed scope, so the
//! job reaches it through the live scope of the person who finalised the
//! record (proved again on every attempt, refused when the account is gone)
//! and the artifact's own retention history names that person. A system
//! principal's retention API in Base Artifacts would name the principal.
use crate::{
    operations::authorize,
    owner::ArtifactOwner,
    retain::{Change, Who},
    retention,
    store::{self, iso},
    Attachment, AttachmentError, Attachments, Refresh, Result, REFRESH_PRINCIPAL,
};
use bilibi_authz::SystemScope;
use bilibi_company::{organisation, WriteError};
use bilibi_database::{Error, MutationTransaction};
use bilibi_queue::execution::{Execution, Handler, HandlerFuture, Identity, Outcome};
use bilibi_tenancy::Scope;
use jiff::Timestamp;
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) const WORKER_ID: &str = "finance/attachments-retention-refresh";
pub(crate) const REFRESH_SERVICE: &str = "finance/attachments.retention_refresh";
const KEYS: [&str; 6] = [
    "tenant_id",
    "company_id",
    "source",
    "document_id",
    "retain_until",
    "finalised_by",
];

/// An exact allowlist of identifiers and the one computed date; any other
/// shape is refused.
pub(crate) fn validate_args(args: Value) -> bilibi_database::Result<Value> {
    let positive = |key: &str| args.get(key).and_then(Value::as_i64).filter(|n| *n > 0);
    let object = args.as_object().filter(|o| o.len() == KEYS.len());
    let source = args.get("source").and_then(Value::as_str);
    let until = args
        .get("retain_until")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<Timestamp>().ok());
    match (object, source, until) {
        (Some(_), Some(source), Some(until))
            if KEYS.iter().all(|key| args.get(*key).is_some())
                && positive("tenant_id").is_some()
                && positive("company_id").is_some()
                && positive("document_id").is_some()
                && positive("finalised_by").is_some()
                && !source.is_empty()
                && source.len() <= 100 =>
        {
            let mut normal = args.clone();
            normal["retain_until"] = Value::String(iso(until));
            Ok(normal)
        }
        _ => Err(Error::Invalid),
    }
}

struct Job {
    tenant: i64,
    company_id: i64,
    source: String,
    document_id: i64,
    until: Timestamp,
    finalised_by: i64,
}
fn job(args: &Value) -> Option<Job> {
    Some(Job {
        tenant: args.get("tenant_id")?.as_i64()?,
        company_id: args.get("company_id")?.as_i64()?,
        source: args.get("source")?.as_str()?.to_owned(),
        document_id: args.get("document_id")?.as_i64()?,
        until: args.get("retain_until")?.as_str()?.parse().ok()?,
        finalised_by: args.get("finalised_by")?.as_i64()?,
    })
}

/// The queue handler of the worker.
pub(crate) struct RefreshJob {
    pub(crate) attachments: Arc<Attachments>,
}
impl Handler for RefreshJob {
    fn perform<'a>(&'a self, execution: &'a Execution, args: Value) -> HandlerFuture<'a> {
        Box::pin(async move {
            match self
                .attachments
                .run_retention_refresh(execution, &args)
                .await
            {
                Ok(()) => Outcome::Completed,
                Err(outcome) => outcome,
            }
        })
    }
}

fn classify(error: AttachmentError) -> Outcome {
    match error {
        AttachmentError::CompanyNotFound => Outcome::Cancel("company_not_found"),
        AttachmentError::CompanyArchived => Outcome::Cancel("company_archived"),
        AttachmentError::SourceUnavailable | AttachmentError::InvalidSource => {
            Outcome::Cancel("source_unavailable")
        }
        AttachmentError::Unauthorized => Outcome::Cancel("actor_refused"),
        _ => Outcome::Retry("retention_refresh_failed"),
    }
}

impl Attachments {
    /// Queues the extension of every standing attachment of the record
    /// `document_id` that `source` describes to what the record's final date
    /// says, for a record whose date can change while it is not final (a draft
    /// invoice re-dated before it is issued). Answers [`Refresh::Queued`], or
    /// [`Refresh::None`] when no attachment needs it. Counts the company's
    /// retention years from the end of the financial year the date falls in;
    /// an attachment already kept that long is left, and none is ever
    /// shortened.
    ///
    /// The caller's own scope confirms the record: `source` must read it for
    /// them and it must be final (`DocumentNotFinal`), and the date's financial
    /// year is the ledger's (`finance.ledger.view`). It needs no attachments
    /// grant. The module calls it once the record is final, inside the
    /// transaction that finalises it, so the job commits with the record and is
    /// never lost; repeating it is safe. The extension itself is the Base Queue
    /// job of `finance-attachments-retention-refresh`.
    pub async fn refresh_retention(
        &self,
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company_id: i64,
        source: &str,
        document_id: i64,
    ) -> Result<Refresh> {
        let registered = self.registered(source)?;
        let document = self
            .source_document(
                crate::SourceClient::Transaction(tx.sql()),
                scope,
                company_id,
                registered,
                document_id,
            )
            .await?;
        if !document.is_final {
            return Err(AttachmentError::DocumentNotFinal);
        }
        organisation::require_live(tx.sql(), scope, company_id)
            .await
            .map_err(|_| AttachmentError::CompanyNotFound)?;
        authorize(tx.sql(), scope, company_id, bilibi_finance_ledger::VIEW).await?;
        let attachments = store::standing_for_record(
            tx.sql(),
            scope.tenant(),
            company_id,
            registered.id,
            document_id,
        )
        .await?;
        let years = self.retention_years(tx.sql(), scope, company_id).await?;
        let until = retention::until(retention::financial_year_end(document.date), years);
        if !attachments.iter().any(|a| until > a.retain_until) {
            return Ok(Refresh::None);
        }
        let system =
            SystemScope::rehydrate(tx.sql(), scope.tenant(), Some(REFRESH_PRINCIPAL)).await?;
        self.queue
            .enqueue_system_within(
                tx,
                &system,
                WORKER_ID,
                json!({
                    "tenant_id": scope.tenant(), "company_id": company_id, "source": registered.id,
                    "document_id": document_id, "retain_until": iso(until), "finalised_by": scope.user(),
                }),
            )
            .await?;
        Ok(Refresh::Queued)
    }

    /// One attempt of the job, as the system principal.
    async fn run_retention_refresh(
        &self,
        execution: &Execution,
        args: &Value,
    ) -> std::result::Result<(), Outcome> {
        let job = job(args).ok_or(Outcome::Cancel("invalid_refresh"))?;
        match execution.identity() {
            Identity::System(system)
                if system.principal() == Some(REFRESH_PRINCIPAL)
                    && system.tenant() == job.tenant => {}
            Identity::System(_) => return Err(Outcome::Cancel("tenant_mismatch")),
            Identity::User(_) => return Err(Outcome::Cancel("actor_unavailable")),
        }
        let registered = self
            .sources
            .get(&job.source)
            .ok_or(Outcome::Cancel("source_unavailable"))?;
        let db = || Outcome::Retry("database_unavailable");
        let client = self.db.get().await.map_err(|_| db())?;
        // The person who finalised the record is the one Base Artifacts proves
        // again; a removed or frozen account ends the job.
        let scope = self
            .authentication
            .authenticate(&**client, job.tenant, job.finalised_by, String::new())
            .await
            .map_err(|error| match error {
                Error::Unauthorized | Error::Forbidden | Error::NotFound => {
                    Outcome::Cancel("actor_refused")
                }
                _ => Outcome::Retry("retention_refresh_failed"),
            })?;
        organisation::require_live(&**client, &scope, job.company_id)
            .await
            .map_err(|_| Outcome::Cancel("company_not_found"))?;
        let standing = store::standing_for_record(
            &**client,
            job.tenant,
            job.company_id,
            registered.id,
            job.document_id,
        )
        .await
        .map_err(|_| db())?;
        drop(client);
        let who = Who::Job(execution, job.finalised_by);
        let reason = format!(
            "{}: retention follows the record's final date",
            registered.id
        );
        for attachment in standing {
            let attachment = self
                .catch_up(&who, &scope, job.company_id, attachment)
                .await
                .map_err(classify)?;
            if job.until <= attachment.retain_until {
                continue;
            }
            self.extend(&who, &scope, &job, &attachment, &reason)
                .await
                .map_err(classify)?;
        }
        Ok(())
    }

    async fn extend(
        &self,
        who: &Who<'_>,
        scope: &Scope,
        job: &Job,
        attachment: &Attachment,
        reason: &str,
    ) -> Result<()> {
        let change = Change::Extend(job.until);
        {
            let Who::Job(execution, _) = who else {
                return Err(AttachmentError::Unauthorized);
            };
            let mut client = self.db.get().await.map_err(Error::from)?;
            let tx = execution
                .begin(&mut client, "finance.attachments.retention_refresh_check")
                .await?;
            organisation::require_writable(tx.sql(), scope, job.company_id, true)
                .await
                .map_err(|error| match error {
                    WriteError::Archived => AttachmentError::CompanyArchived,
                    WriteError::NotFound => AttachmentError::CompanyNotFound,
                    _ => AttachmentError::Failed(Error::Internal),
                })?;
            let current = store::fetch(tx.sql(), job.tenant, job.company_id, attachment.id)
                .await?
                .ok_or(AttachmentError::AttachmentNotFound)?;
            tx.rollback().await?;
            if job.until <= current.retain_until {
                return Ok(());
            }
        }
        match self
            .apply(
                scope,
                job.company_id,
                &ArtifactOwner::refresh(),
                attachment,
                change,
                reason,
            )
            .await
        {
            Ok(()) | Err(AttachmentError::RetentionNotExtended) => {}
            Err(error) => return Err(error),
        }
        self.record(who, job.company_id, attachment.id, change, reason)
            .await?;
        Ok(())
    }
}
