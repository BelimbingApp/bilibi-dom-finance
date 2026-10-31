//! Changing the retention of one attachment: a later retain-until and a legal
//! hold. Base Artifacts changes first, outside any transaction: what it
//! enforces is the fact, and the event follows it. A short transaction
//! beforehand holds the company lock, so an archived company is refused
//! before Base is touched, and checks the change against the events; the
//! event is then appended without the company lock, since the change is
//! already true in storage. Reads bring the events up to Base's state
//! ([`Attachments::catch_up`]), so a gap closes on the next read.
use crate::{
    operations::{authorize, event_change},
    owner::ArtifactOwner,
    retention,
    store::{self, NewEvent},
    Attachment, AttachmentError, Attachments, Result, MANAGE_RETENTION, VIEW,
};
use bilibi_artifacts::RetentionEvent;
use bilibi_database::{Error, MutationTransaction};
use bilibi_party_ids::OrganisationId;
use bilibi_queue::Execution;
use bilibi_tenancy::Scope;
use jiff::{civil::Date, Timestamp};
use std::time::SystemTime;
use tokio_postgres::Client;

/// What a retention change does to the artifact in Base.
#[derive(Clone, Copy)]
pub(crate) enum Change {
    Extend(Timestamp),
    PlaceHold,
    ReleaseHold,
}
impl Change {
    fn event(self) -> &'static str {
        match self {
            Self::Extend(_) => "retention_extended",
            Self::PlaceHold => "hold_placed",
            Self::ReleaseHold => "hold_released",
        }
    }
    fn retain_until(self) -> Option<Timestamp> {
        match self {
            Self::Extend(until) => Some(until),
            _ => None,
        }
    }
    /// Decided from the events before Base is touched.
    fn permitted(self, attachment: &Attachment) -> Result<()> {
        match self {
            Self::Extend(until) if until > attachment.retain_until => Ok(()),
            Self::Extend(_) => Err(AttachmentError::RetentionNotExtended),
            Self::PlaceHold if attachment.on_hold => Err(AttachmentError::AlreadyOnHold),
            Self::ReleaseHold if !attachment.on_hold => Err(AttachmentError::NotOnHold),
            _ => Ok(()),
        }
    }
    /// Whether the events already show the change.
    fn shown(self, attachment: &Attachment) -> bool {
        match self {
            Self::Extend(until) => until <= attachment.retain_until,
            Self::PlaceHold => attachment.on_hold,
            Self::ReleaseHold => !attachment.on_hold,
        }
    }
}

/// Who a retention event and its transaction are attributed to.
#[derive(Clone, Copy)]
pub(crate) enum Who<'a> {
    /// A person acting now.
    Scope(&'a Scope),
    /// The retention refresh job, running as its principal for the person who
    /// finalised the record.
    Job(&'a Execution, i64),
}
type Actor<'a> = (i64, Option<i64>, Option<&'a str>);
impl Who<'_> {
    async fn begin<'c>(
        &self,
        attachments: &Attachments,
        client: &'c mut Client,
        action: &str,
    ) -> Result<MutationTransaction<'c>> {
        Ok(match self {
            Self::Scope(scope) => {
                attachments
                    .mutations
                    .begin(client, scope.mutation_context(action))
                    .await?
            }
            Self::Job(execution, _) => execution.begin(client, action).await?,
        })
    }
    fn actor(&self) -> Actor<'static> {
        match self {
            Self::Scope(scope) => {
                let (user, impersonator) = crate::operations::actor_ids(scope);
                (user, impersonator, None)
            }
            Self::Job(_, user) => (*user, None, Some(crate::REFRESH_PRINCIPAL)),
        }
    }
}

impl Attachments {
    /// Moves an attachment's retain-until later, to the end of `retain_until`
    /// (a date) in UTC, with a `reason`
    /// (`finance.attachments.retention.manage`, and `view`): an equal or
    /// earlier date than the attachment's own is `RetentionNotExtended`. Base
    /// Artifacts moves the artifact's retain-until first and keeps the change
    /// with its own history; then `retention_extended` is recorded here. If
    /// recording fails after Base applied the change, the answer is
    /// `RecordedInStorage` and the next read of the attachment records it. Any
    /// attachment, standing or not.
    pub async fn extend_retention(
        &self,
        scope: &Scope,
        company_id: i64,
        attachment_id: i64,
        retain_until: Date,
        reason: &str,
    ) -> Result<Attachment> {
        self.manage(
            scope,
            company_id,
            attachment_id,
            reason,
            Change::Extend(retention::end_of_day(retain_until)),
        )
        .await
    }

    /// Places a legal hold on an attachment's bytes with a `reason`: Base
    /// Artifacts refuses their deletion past retain-until until the hold is
    /// released (`AlreadyOnHold` for a second hold). Recorded as
    /// [`Attachments::extend_retention`] is.
    pub async fn place_hold(
        &self,
        scope: &Scope,
        company_id: i64,
        attachment_id: i64,
        reason: &str,
    ) -> Result<Attachment> {
        self.manage(scope, company_id, attachment_id, reason, Change::PlaceHold)
            .await
    }

    /// Releases an attachment's legal hold with a `reason` (`NotOnHold` without
    /// one).
    pub async fn release_hold(
        &self,
        scope: &Scope,
        company_id: i64,
        attachment_id: i64,
        reason: &str,
    ) -> Result<Attachment> {
        self.manage(
            scope,
            company_id,
            attachment_id,
            reason,
            Change::ReleaseHold,
        )
        .await
    }

    async fn manage(
        &self,
        scope: &Scope,
        company_id: i64,
        attachment_id: i64,
        reason: &str,
        change: Change,
    ) -> Result<Attachment> {
        let attachment = {
            let client = self.db.get().await.map_err(Error::from)?;
            authorize(&**client, scope, company_id, MANAGE_RETENTION).await?;
            authorize(&**client, scope, company_id, VIEW).await?;
            self.readable_attachment(&client, scope, company_id, attachment_id)
                .await?
                .0
        };
        let reason = event_reason(reason)?;
        let who = Who::Scope(scope);
        let attachment = self.catch_up(&who, scope, company_id, attachment).await?;
        // A short transaction under the company lock: an archived company is
        // refused before Base is touched.
        {
            let mut client = self.db.get().await.map_err(Error::from)?;
            let tx = self
                .write(
                    &mut client,
                    scope,
                    company_id,
                    MANAGE_RETENTION,
                    "finance.attachments.retention_check",
                )
                .await?;
            let current = self
                .any_attachment(tx.sql(), scope, company_id, attachment.id)
                .await?;
            let outcome = change.permitted(&current);
            tx.rollback().await?;
            outcome?;
        }
        self.apply(
            scope,
            company_id,
            &ArtifactOwner::user(),
            &attachment,
            change,
            &reason,
        )
        .await?;
        self.record(&who, company_id, attachment.id, change, &reason)
            .await
    }

    /// The change in Base Artifacts, which re-authorizes it.
    pub(crate) async fn apply(
        &self,
        scope: &Scope,
        company_id: i64,
        owner: &ArtifactOwner,
        attachment: &Attachment,
        change: Change,
        reason: &str,
    ) -> Result<()> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let company =
            OrganisationId::new(company_id).map_err(|_| AttachmentError::CompanyNotFound)?;
        let documents = self.artifacts.for_owner(scope, company, owner);
        let id = &attachment.artifact_id;
        match change {
            Change::Extend(until) => {
                documents
                    .retain(&mut client, id, SystemTime::from(until), Some(reason))
                    .await?
            }
            Change::PlaceHold => documents.place_hold(&mut client, id, Some(reason)).await?,
            Change::ReleaseHold => {
                documents
                    .release_hold(&mut client, id, Some(reason))
                    .await?
            }
        };
        Ok(())
    }

    /// Records the event of a change Base applied, unless the events already
    /// show it (a writer and a reader catching up at the same moment record it
    /// once).
    pub(crate) async fn record(
        &self,
        who: &Who<'_>,
        company_id: i64,
        attachment_id: i64,
        change: Change,
        reason: &str,
    ) -> Result<Attachment> {
        let (user, impersonator, principal) = who.actor();
        self.record_as(
            who,
            company_id,
            attachment_id,
            change,
            reason,
            (user, impersonator, principal),
        )
        .await
        .map_err(|error| match error {
            failed @ AttachmentError::Failed(_) => {
                AttachmentError::RecordedInStorage(failed.to_string())
            }
            other => AttachmentError::RecordedInStorage(other.to_string()),
        })
    }

    async fn record_as(
        &self,
        who: &Who<'_>,
        company_id: i64,
        attachment_id: i64,
        change: Change,
        reason: &str,
        (user, impersonator, principal): Actor<'_>,
    ) -> Result<Attachment> {
        let tenant = match who {
            Who::Scope(scope) => scope.tenant(),
            Who::Job(execution, _) => execution.tenant(),
        };
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = who
            .begin(
                self,
                &mut client,
                &format!("finance.attachments.{}", change.event()),
            )
            .await?;
        if !store::lock(tx.sql(), tenant, company_id, attachment_id).await? {
            return Err(AttachmentError::AttachmentNotFound);
        }
        let locked = store::fetch(tx.sql(), tenant, company_id, attachment_id)
            .await?
            .ok_or(AttachmentError::AttachmentNotFound)?;
        if !change.shown(&locked) {
            let (event, reason) = (change.event(), reason.to_owned());
            let until = change.retain_until();
            let principal = principal.map(str::to_owned);
            tx.mutate(move |sql| {
                Box::pin(async move {
                    let (id, json) = store::insert_event(
                        sql,
                        tenant,
                        company_id,
                        &NewEvent {
                            attachment_id,
                            event,
                            replacement_id: None,
                            retain_until: until,
                            reason: Some(&reason),
                            user,
                            impersonator,
                            principal: principal.as_deref(),
                        },
                    )
                    .await?;
                    Ok(((), vec![event_change(id, json)]))
                })
            })
            .await?;
        }
        let current = store::fetch(tx.sql(), tenant, company_id, attachment_id)
            .await?
            .ok_or(AttachmentError::AttachmentNotFound)?;
        tx.commit().await?;
        Ok(self.present(current))
    }

    /// Appends the events Base Artifacts holds and this module does not: a
    /// retain-until later than the events' or a legal hold they do not show,
    /// each carrying what Base recorded for the latest such change (its actor,
    /// impersonator or system principal, and reason), never the reader. A
    /// failed read of Base changes nothing.
    pub(crate) async fn catch_up(
        &self,
        who: &Who<'_>,
        scope: &Scope,
        company_id: i64,
        attachment: Attachment,
    ) -> Result<Attachment> {
        let Ok(history) = self.history(who, scope, company_id, &attachment).await else {
            return Ok(attachment);
        };
        self.append_missing(who, company_id, attachment, history)
            .await
    }

    pub(crate) async fn history(
        &self,
        who: &Who<'_>,
        scope: &Scope,
        company_id: i64,
        attachment: &Attachment,
    ) -> std::result::Result<Vec<RetentionEvent>, AttachmentError> {
        let client = self.db.get().await.map_err(Error::from)?;
        let company =
            OrganisationId::new(company_id).map_err(|_| AttachmentError::CompanyNotFound)?;
        let owner = match who {
            Who::Scope(_) => ArtifactOwner::user(),
            Who::Job(..) => ArtifactOwner::refresh(),
        };
        Ok(self
            .artifacts
            .for_owner(scope, company, &owner)
            .retention_history(&client, &attachment.artifact_id)
            .await?)
    }

    pub(crate) async fn append_missing(
        &self,
        who: &Who<'_>,
        company_id: i64,
        attachment: Attachment,
        history: Vec<RetentionEvent>,
    ) -> Result<Attachment> {
        let missing = missing_events(&attachment, &history);
        if missing.is_empty() {
            return Ok(attachment);
        }
        for (change, reason, actor) in missing {
            let actor = (actor.0, actor.1, actor.2.as_deref());
            // A gap that cannot be recorded now closes on a later read.
            let _ = self
                .record_as(who, company_id, attachment.id, change, &reason, actor)
                .await;
        }
        let client = self.db.get().await.map_err(Error::from)?;
        let tenant = match who {
            Who::Scope(scope) => scope.tenant(),
            Who::Job(execution, _) => execution.tenant(),
        };
        Ok(store::fetch(&**client, tenant, company_id, attachment.id)
            .await?
            .map_or(attachment, |current| self.present(current)))
    }
}

type Recorded = (Change, String, (i64, Option<i64>, Option<String>));

fn missing_events(attachment: &Attachment, history: &[RetentionEvent]) -> Vec<Recorded> {
    let Some(base) = history.last() else {
        return Vec::new();
    };
    let recorded = |source: &RetentionEvent| {
        source
            .reason
            .clone()
            .unwrap_or_else(|| "recorded from Base Artifacts' retention history".into())
    };
    let actor = |source: &RetentionEvent| {
        (
            source.actor_id,
            source.impersonator_id,
            source.system_principal.clone(),
        )
    };
    let last_of = |events: &[&str]| history.iter().rfind(|e| events.contains(&e.event.as_str()));
    let mut missing = Vec::new();
    if let Some(until) = pg_timestamp(&base.retain_until) {
        if until > attachment.retain_until {
            let source = last_of(&["retention_extended"]).unwrap_or(base);
            missing.push((Change::Extend(until), recorded(source), actor(source)));
        }
    }
    if base.legal_hold != attachment.on_hold {
        let source = last_of(&["hold_placed", "hold_released"]).unwrap_or(base);
        let change = if base.legal_hold {
            Change::PlaceHold
        } else {
            Change::ReleaseHold
        };
        missing.push((change, recorded(source), actor(source)));
    }
    missing
}

/// A PostgreSQL `timestamptz::text` (`2033-12-31 23:59:59.999999+00`).
pub(crate) fn pg_timestamp(text: &str) -> Option<Timestamp> {
    let mut text = text.replace(' ', "T");
    if let Some(index) = text.rfind(['+', '-']).filter(|i| *i > 10) {
        if text.len() - index == 3 {
            text.push_str(":00");
        }
    }
    text.parse().ok()
}

/// A reason of at most 255 characters that says something.
fn event_reason(reason: &str) -> Result<String> {
    let trimmed = reason.trim();
    if trimmed.is_empty() || reason.len() > 255 {
        return Err(AttachmentError::ReasonRequired);
    }
    Ok(reason.to_owned())
}
