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
