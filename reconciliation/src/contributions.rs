//! What the reconciliation module adds to the shared catalogs (Bilimbi
//! `Bilimbi.Finance.Reconciliation.Contributions`): the eight capability keys
//! verbatim, the verbs `import` and `finalise`, and the `tenant_owner` role
//! holding all of them. The `finance` domain and the `post` verb are the
//! ledger's, `view` and `manage` are Base's.
//!
//! `TODO(menu)`: Bilimbi's menu item `finance.reconciliation` ("Bank
//! reconciliation", route `/finance/reconciliation`) lands with the bank
//! accounts page; a menu item must not point at a route that is not mounted.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};

/// The capability every reconciliation read asks for.
pub const VIEW: &str = "finance.reconciliation.view";
/// Adds and changes bank accounts.
pub const MANAGE_BANK_ACCOUNTS: &str = "finance.reconciliation.bank-accounts.manage";
/// Imports bank statements.
pub const IMPORT_STATEMENTS: &str = "finance.reconciliation.statements.import";
/// Matches and unmatches statement lines with ledger entries.
pub const MANAGE_MATCHES: &str = "finance.reconciliation.matches.manage";
/// Opens and abandons reconciliations.
pub const MANAGE_RECONCILIATIONS: &str = "finance.reconciliation.reconciliations.manage";
/// Finalises a reconciliation.
pub const FINALISE_RECONCILIATIONS: &str = "finance.reconciliation.reconciliations.finalise";
/// Posts a statement line as a charge or interest.
pub const POST_STATEMENT_LINES: &str = "finance.reconciliation.statement-lines.post";
/// Reverses the entry a statement line posted.
pub const MANAGE_STATEMENT_LINES: &str = "finance.reconciliation.statement-lines.manage";

pub const CAPABILITIES: [&str; 8] = [
    VIEW,
    MANAGE_BANK_ACCOUNTS,
    IMPORT_STATEMENTS,
    MANAGE_MATCHES,
    MANAGE_RECONCILIATIONS,
    FINALISE_RECONCILIATIONS,
    POST_STATEMENT_LINES,
    MANAGE_STATEMENT_LINES,
];

macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}

inventory::submit! { Contribution::new(VOCABULARY, "finance/reconciliation", provenance!(), &VocabularyContribution {
    domains: &[],
    verbs: &["import", "finalise"],
    capabilities: &[
        C::tenant(VIEW), C::tenant(MANAGE_BANK_ACCOUNTS), C::tenant(IMPORT_STATEMENTS), C::tenant(MANAGE_MATCHES),
        C::tenant(MANAGE_RECONCILIATIONS), C::tenant(FINALISE_RECONCILIATIONS), C::tenant(POST_STATEMENT_LINES),
        C::tenant(MANAGE_STATEMENT_LINES),
    ],
    roles: &[RoleDefinition {
        code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly",
        grant_all: false, capabilities: &CAPABILITIES,
    }],
    system_principals: &[],
}) }
