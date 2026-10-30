//! Customer relationship wording over Finance's shared Core company picker.
use super::Pages;
use bilibi_database::Result;
pub(super) use bilibi_finance_ledger::{PartyRelation as Relation, PartySubmission as Submitted};
use bilibi_tenancy::Scope;
pub(super) async fn relation(
    pages: &Pages,
    scope: &Scope,
    kind: &str,
    target: i64,
) -> Result<Relation> {
    bilibi_finance_ledger::party_relation(&pages.party_picker(), scope, kind, target).await
}
pub(super) async fn record_relationship(
    pages: &Pages,
    scope: &Scope,
    credential: &str,
    event: &str,
    kind: &str,
    target: i64,
) -> Result<std::result::Result<(), String>> {
    bilibi_finance_ledger::record_party_relationship(
        &pages.party_picker(),
        scope,
        credential,
        event,
        kind,
        target,
    )
    .await
}
