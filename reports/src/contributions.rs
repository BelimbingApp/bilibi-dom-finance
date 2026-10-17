//! What the module adds to the shared catalogs (Bilimbi
//! `Bilimbi.Finance.Reports.Contributions`): its two capability keys verbatim
//! and the `tenant_owner` role holding both. Reports read ledger figures, so
//! they stand on the ledger's capabilities: viewing them is viewing the ledger
//! (`finance.ledger.view`), and arranging them is part of keeping the chart of
//! accounts (`finance.ledger.accounts.manage`, the sections setting's editor
//! capability, in `configuration.rs`). Menu items arrive with the pages that
//! own them.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};

/// Issues an immutable report snapshot.
pub const ISSUE: &str = "finance.reports.issue";
/// Generates and downloads a company auditor pack.
pub const EXPORT: &str = "finance.reports.reports-export";

pub const CAPABILITIES: [&str; 2] = [ISSUE, EXPORT];

macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}
pub(crate) use provenance;

// Bilimbi declares the verb `reports-export` and relies on a sibling for
// `issue`; Bilibi merges verbs across modules, so both are declared here and
// the module needs no sibling to have declared one. The keys are unchanged.
inventory::submit! { Contribution::new(VOCABULARY, "finance/reports", provenance!(), &VocabularyContribution {
    domains: &["finance"],
    verbs: &["issue", "reports-export"],
    capabilities: &[C::tenant(ISSUE), C::tenant(EXPORT)],
    roles: &[RoleDefinition {
        code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly",
        grant_all: false, capabilities: &CAPABILITIES,
    }],
    system_principals: &[],
}) }

// Bilimbi reports/priv/web_routes.exs and Reports.Contributions.
use bilibi_module_registry::contributions::{Access, Method, Route, ROUTES};
inventory::submit! { Contribution::new(ROUTES, "finance/reports.index", provenance!(), &Route {
    method: Method::Get, path: "/finance/reports", service: "finance/reports.pages",
    operation: "reports", access: Access::Capability(crate::VIEW), framed: true,
}) }
inventory::submit! { Contribution::new(ROUTES, "finance/reports.activity", provenance!(), &Route {
    method: Method::Get, path: "/finance/reports/accounts/{account_id}", service: "finance/reports.pages",
    operation: "activity", access: Access::Capability(crate::VIEW), framed: true,
}) }
bilibi_menu::contribute!(
    "finance/reports",
    bilibi_menu::MenuItem::page("finance.reports", "Reports", "/finance/reports")
        .under("finance")
        .requires(bilibi_menu::Requirement::Capability(crate::VIEW))
        .order(20)
);

// Bilimbi Reports.Web.{BookkeepingLive,ApprovalExceptionsLive} routes/menu.
inventory::submit! { Contribution::new(ROUTES, "finance/reports.bookkeeping", provenance!(), &Route {
    method: Method::Get, path: "/finance/reports/bookkeeping", service: "finance/reports.control-pages", operation: "bookkeeping", access: Access::Capability(crate::VIEW), framed: true,
}) }
inventory::submit! { Contribution::new(ROUTES, "finance/reports.approval-exceptions", provenance!(), &Route {
    method: Method::Get, path: "/finance/reports/approval-exceptions", service: "finance/reports.control-pages", operation: "approval-exceptions", access: Access::Capability(crate::VIEW), framed: true,
}) }
bilibi_menu::contribute!(
    "finance/reports",
    bilibi_menu::MenuItem::page(
        "finance.reports.bookkeeping",
        "Bookkeeping",
        "/finance/reports/bookkeeping"
    )
    .under("finance")
    .requires(bilibi_menu::Requirement::Capability(crate::VIEW))
    .order(22)
);
bilibi_menu::contribute!(
    "finance/reports",
    bilibi_menu::MenuItem::page(
        "finance.reports.approval-exceptions",
        "Approval exceptions",
        "/finance/reports/approval-exceptions"
    )
    .under("finance")
    .requires(bilibi_menu::Requirement::Capability(crate::VIEW))
    .order(21)
);
