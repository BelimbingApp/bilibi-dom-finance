//! Mirrors Bilimbi Payroll.Contributions and priv/web_routes.exs.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};
pub const VIEW: &str = "finance.payroll.view";
pub const MANAGE_SETTINGS: &str = "finance.payroll.settings.manage";
pub const POST_POSTINGS: &str = "finance.payroll.postings.post";
pub const CAPABILITIES: [&str; 3] = [VIEW, MANAGE_SETTINGS, POST_POSTINGS];
inventory::submit! { Contribution::new(VOCABULARY, MODULE, Provenance { module: MODULE, source: file!(), line: line!() }, &VocabularyContribution {
    domains: &[], verbs: &[], capabilities: &[C::tenant(VIEW), C::tenant(MANAGE_SETTINGS), C::tenant(POST_POSTINGS)],
    roles: &[RoleDefinition { code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly", grant_all: false, capabilities: &CAPABILITIES }],
    system_principals: &[],
}) }

use bilibi_module_registry::contributions::{Access, Method, Route, ROUTES};
inventory::submit! { Contribution::new(ROUTES, "finance/payroll.postings", Provenance { module: MODULE, source: file!(), line: line!() }, &Route {
    method: Method::Get, path: "/finance/payroll", service: "finance/payroll.pages", operation: "postings", access: Access::Capability(VIEW), framed: true,
}) }
inventory::submit! { Contribution::new(ROUTES, "finance/payroll.setup", Provenance { module: MODULE, source: file!(), line: line!() }, &Route {
    method: Method::Get, path: "/finance/payroll/setup", service: "finance/payroll.pages", operation: "setup", access: Access::Capability(VIEW), framed: true,
}) }
// Menu group (a bookkeeper's section of the Finance menu); its pages sit under it.
bilibi_menu::contribute!(
    "finance/payroll",
    bilibi_menu::MenuItem::container("finance.group.payroll", "Payroll")
        .under("finance")
        .order(70)
);

bilibi_menu::contribute!(
    "finance/payroll",
    bilibi_menu::MenuItem::page("finance.payroll", "Payroll and claims", "/finance/payroll")
        .under("finance.group.payroll")
        .icon("users")
        .requires(bilibi_menu::Requirement::Capability(VIEW))
        .order(1)
);

// ADR 01 gap, listed so it stays visible: these pages still write through
// the facade rather than an operation contract (`base/agent_api` page).
inventory::submit! { bilibi_module_registry::Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, "finance/payroll.pages", bilibi_module_registry::Provenance { module: MODULE, source: file!(), line: line!() }, &bilibi_module_registry::contributions::PageWrites {
    service: "finance/payroll.pages",
    contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Posting runs and claims, reversals, mappings and settings write through the Payroll facade; no contract yet." },
}) }
