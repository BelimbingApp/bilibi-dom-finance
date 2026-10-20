//! What the ledger adds to the shared catalogs (Bilimbi
//! `Bilimbi.Finance.Ledger.Contributions`): the `finance` authorization
//! domain, its verbs, the twelve capability keys verbatim, and the
//! `tenant_owner` role holding all of them. H1 agent operations are in
//! `agent_operations.rs`; its pages and their menu items are in `web/`.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};

/// The capability every ledger read asks for.
pub const VIEW: &str = "finance.ledger.view";
/// Opens a book and maintains the chart of accounts.
pub const MANAGE_ACCOUNTS: &str = "finance.ledger.accounts.manage";
/// Posts and reverses journal entries.
pub const POST_ENTRIES: &str = "finance.ledger.entries.post";
/// Opens fiscal years and closes periods and years.
pub const MANAGE_PERIODS: &str = "finance.ledger.periods.manage";
/// The privileged capability that reopens a closed period or fiscal year.
pub const REOPEN_PERIODS: &str = "finance.ledger.periods.reopen";
/// Locks a closed period for good.
pub const LOCK_PERIODS: &str = "finance.ledger.periods.lock";
/// Turns the company's separation of duties on or off.
pub const MANAGE_DUTY_SEPARATION: &str = "finance.ledger.duty-separation.manage";
/// Turns the company's controlled bookkeeping on or off.
pub const MANAGE_CONTROLLED_BOOKKEEPING: &str = "finance.ledger.controlled-bookkeeping.manage";
/// Records an independent review of an entry recorded late.
pub const REVIEW_LATE_RECORDINGS: &str = "finance.ledger.late-recordings.review";
/// Submits a manual journal for approval.
pub const SUBMIT_JOURNALS: &str = "finance.ledger.journals.submit";
/// Approves (posting it) or rejects a submitted journal.
pub const APPROVE_JOURNALS: &str = "finance.ledger.journals.approve";
/// Records an independent review of an approval exception.
pub const REVIEW_EXCEPTIONS: &str = "finance.ledger.approval-exceptions.review";

pub const CAPABILITIES: [&str; 12] = [
    VIEW,
    MANAGE_ACCOUNTS,
    POST_ENTRIES,
    MANAGE_PERIODS,
    REOPEN_PERIODS,
    LOCK_PERIODS,
    MANAGE_DUTY_SEPARATION,
    MANAGE_CONTROLLED_BOOKKEEPING,
    REVIEW_LATE_RECORDINGS,
    SUBMIT_JOURNALS,
    APPROVE_JOURNALS,
    REVIEW_EXCEPTIONS,
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

// Bilimbi declares the verbs `post`, `reopen` and `lock` and relies on Base
// for `view`, `manage`, `submit`, `approve` and `review`. Bilibi merges verbs
// across every module, and no Platform module declares `submit` or `review`,
// so the ledger declares those two as well; the keys are unchanged.
inventory::submit! { Contribution::new(VOCABULARY, "finance/ledger", provenance!(), &VocabularyContribution {
    domains: &["finance"],
    verbs: &["post", "reopen", "lock", "submit", "review"],
    capabilities: &[
        C::tenant(VIEW), C::tenant(MANAGE_ACCOUNTS), C::tenant(POST_ENTRIES), C::tenant(MANAGE_PERIODS),
        C::tenant(REOPEN_PERIODS), C::tenant(LOCK_PERIODS), C::tenant(MANAGE_DUTY_SEPARATION),
        C::tenant(MANAGE_CONTROLLED_BOOKKEEPING), C::tenant(REVIEW_LATE_RECORDINGS), C::tenant(SUBMIT_JOURNALS),
        C::tenant(APPROVE_JOURNALS), C::tenant(REVIEW_EXCEPTIONS),
    ],
    roles: &[RoleDefinition {
        code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly",
        grant_all: false, capabilities: &CAPABILITIES,
    }],
    system_principals: &[],
}) }
