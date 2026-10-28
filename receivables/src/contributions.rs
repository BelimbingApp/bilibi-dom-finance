//! Bilimbi Receivables.Contributions, capability keys verbatim.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};

pub const VIEW: &str = "finance.receivables.view";
pub const MANAGE_SETUP: &str = "finance.receivables.setup.manage";
pub const MANAGE_CUSTOMERS: &str = "finance.receivables.customers.manage";
pub const MANAGE_DOCUMENTS: &str = "finance.receivables.documents.manage";
pub const ISSUE: &str = "finance.receivables.documents.issue";
pub const POST_PAYMENTS: &str = "finance.receivables.payments.post";
pub const SUBMIT_WRITE_OFFS: &str = "finance.receivables.write-offs.submit";
pub const APPROVE_WRITE_OFFS: &str = "finance.receivables.write-offs.approve";
pub const CAPABILITIES: [&str; 8] = [
    VIEW,
    MANAGE_SETUP,
    MANAGE_CUSTOMERS,
    MANAGE_DOCUMENTS,
    ISSUE,
    POST_PAYMENTS,
    SUBMIT_WRITE_OFFS,
    APPROVE_WRITE_OFFS,
];

inventory::submit! { Contribution::new(VOCABULARY, MODULE, Provenance { module: MODULE, source: file!(), line: line!() }, &VocabularyContribution {
    domains: &["finance"], verbs: &["issue"],
    capabilities: &[C::tenant(VIEW), C::tenant(MANAGE_SETUP), C::tenant(MANAGE_CUSTOMERS), C::tenant(MANAGE_DOCUMENTS), C::tenant(ISSUE), C::tenant(POST_PAYMENTS), C::tenant(SUBMIT_WRITE_OFFS), C::tenant(APPROVE_WRITE_OFFS)],
    roles: &[RoleDefinition { code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly", grant_all: false, capabilities: &CAPABILITIES }],
    system_principals: &[],
}) }
