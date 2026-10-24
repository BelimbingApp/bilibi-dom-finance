//! Bilimbi Reports.ArtifactOwner. A stored pack is as sensitive as every source.
use crate::{EXPORT, ISSUE, MODULE, VIEW};
use bilibi_artifacts::{Operation, Owner, Reference};
use bilibi_company::Directory;
use bilibi_database::Error;
use bilibi_party_ids::OrganisationId;
use bilibi_tenancy::{OrganisationDirectory, ProofClient, ProofFuture, Scope};

pub(crate) struct ArtifactOwner;

pub(crate) const SOURCE_VIEWS: &[&str] = &[
    VIEW,
    bilibi_finance_attachments::VIEW,
    bilibi_finance_receivables::VIEW,
    bilibi_finance_payables::VIEW,
    bilibi_finance_reconciliation::VIEW,
    bilibi_finance_einvoice::VIEW,
    bilibi_finance_payroll::VIEW,
    bilibi_finance_sst::VIEW,
    bilibi_finance_fixed_assets::VIEW,
];

pub(crate) async fn authorize_sources(
    client: &dyn ProofClient,
    scope: &Scope,
    company: OrganisationId,
) -> Result<(), Error> {
    for capability in SOURCE_VIEWS {
        Directory.target(client, scope, company, capability).await?;
    }
    Ok(())
}

impl Owner for ArtifactOwner {
    fn owner_id(&self) -> &'static str {
        MODULE
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
            if operation != Operation::Purge
                && !reference.is_some_and(|r| {
                    r.kind == "auditor-pack" && uuid::Uuid::parse_str(&r.subject).is_ok()
                })
            {
                return Err(Error::Forbidden);
            }
            let exporting = matches!(operation, Operation::Create | Operation::Read);
            let target = Directory
                .target(
                    client,
                    scope,
                    company,
                    if exporting { EXPORT } else { ISSUE },
                )
                .await?;
            if target.archived {
                return Err(Error::Forbidden);
            }
            if exporting {
                authorize_sources(client, scope, company).await?;
            }
            Ok(())
        })
    }
}
