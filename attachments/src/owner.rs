//! The module's adapter for Base Artifacts (Bilimbi `ArtifactOwner`): who may
//! store, read, delete and retain a source document's bytes.
//!
//! Every operation re-asks the capability the facade asks for the same
//! action, for the company. Storing (`Create`) needs
//! `finance.attachments.create`; retaining at capture or later needs it or
//! `finance.attachments.retention.manage`; reading needs
//! `finance.attachments.view`; deleting and purging need
//! `finance.attachments.delete`, though a records-grade document is never
//! deleted before its retain-until, and nothing here purges one.
//!
//! The one exception is [`ArtifactOwner::refresh`]: the lengthening that
//! follows a record's finalisation runs as a Base Queue job under the system
//! principal (`refresh.rs`), whose owner accepts `Retain` (and `Read`, for the
//! retention history it reconciles) without an
//! attachments grant, so the person who finalised the record need hold none.
//! That owner is built only by the job, after the principal's own grant was
//! proven.
use crate::{CREATE, DELETE, MANAGE_RETENTION, VIEW};
use bilibi_artifacts::{Operation, Owner, Reference};
use bilibi_company::Directory;
use bilibi_database::Error;
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::{OrganisationDirectory, ProofClient, ProofFuture, Scope};

/// The stable Artifacts owner id.
pub const OWNER_ID: &str = "finance/attachments";
/// The artifact reference kind of a source document; its subject is the
/// source id.
pub(crate) const KIND: &str = "source-document";

pub(crate) struct ArtifactOwner {
    refresh: bool,
}
impl ArtifactOwner {
    /// The adapter of a user's own calls.
    pub(crate) const fn user() -> Self {
        Self { refresh: false }
    }
    /// The adapter of the retention refresh job.
    pub(crate) const fn refresh() -> Self {
        Self { refresh: true }
    }
}

pub(crate) fn reference(source_id: &str) -> Reference {
    Reference {
        subject: source_id.to_owned(),
        kind: KIND.to_owned(),
    }
}

impl Owner for ArtifactOwner {
    fn owner_id(&self) -> &'static str {
        OWNER_ID
    }
    fn authorize<'a>(
        &'a self,
        client: &'a dyn ProofClient,
        scope: &'a Scope,
        company: OrganisationId,
        operation: Operation,
        reference: Option<&'a Reference>,
    ) -> ProofFuture<'a, ()> {
        Box::pin(async move {
            if operation != Operation::Purge && reference.is_none_or(|r| r.kind != KIND) {
                return Err(Error::Forbidden);
            }
            let wanted: &[&str] = match operation {
                Operation::Create => &[CREATE],
                Operation::Read => &[VIEW],
                Operation::Retain => &[CREATE, MANAGE_RETENTION],
                Operation::Delete | Operation::Purge => &[DELETE],
            };
            if self.refresh && matches!(operation, Operation::Retain | Operation::Read) {
                return live(client, scope, company).await;
            }
            for capability in wanted {
                if Directory
                    .target(client, scope, company, capability)
                    .await
                    .is_ok()
                {
                    return Ok(());
                }
            }
            Err(Error::Forbidden)
        })
    }
}

async fn live(
    client: &dyn ProofClient,
    scope: &Scope,
    company: OrganisationId,
) -> bilibi_database::Result<()> {
    Directory
        .lookup(client, scope, company)
        .await?
        .map(|_| ())
        .ok_or(Error::Forbidden)
}
