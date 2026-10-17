//! Mirrors Bilimbi Payroll.Contributions; pages/menu ship in NY2.
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
