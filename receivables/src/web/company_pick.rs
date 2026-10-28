//! Customer relationship wording over Finance's shared Core company picker.
use super::Pages;
use bilibi_database::Result;
pub(super) use bilibi_finance_ledger::{
    ChosenCompany as Chosen, CompanyChooser as Chooser, CompanyRelation as Relation,
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
pub(super) fn relationship_flash(
    done: &str,
    recorded: &std::result::Result<(), String>,
) -> (bilibi_ui::StatusKind, String) {
    bilibi_finance_ledger::company_relationship_flash("customer", done, recorded)
}
