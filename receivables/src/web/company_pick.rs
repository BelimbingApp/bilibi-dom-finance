//! Customer relationship wording over Finance's shared Core company picker.
use super::Pages;
use bilibi_database::Result;
pub(super) use bilibi_finance_ledger::{
    CompanyChooser as Chooser, CompanyRelation as Relation, CompanySubmission as Submitted,
};
use bilibi_tenancy::Scope;
pub(super) async fn relation(pages: &Pages, scope: &Scope, target: i64) -> Result<Relation> {
    bilibi_finance_ledger::company_relation(&pages.company_picker(), scope, target).await
}
pub(super) async fn record_relationship(
    pages: &Pages,
    scope: &Scope,
    credential: &str,
    event: &str,
    target: i64,
) -> Result<std::result::Result<(), String>> {
    bilibi_finance_ledger::record_company_relationship(
        &pages.company_picker(),
        scope,
        credential,
        event,
        target,
    )
    .await
}
