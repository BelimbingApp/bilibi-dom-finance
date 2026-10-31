//! What the attachments module adds to the shared catalogs (Bilimbi
//! `Bilimbi.Finance.Attachments.Contributions`): its four capability keys
//! verbatim, the `tenant_owner` role holding them, the system principal of
//! the retention refresh and its queue worker. The "Source documents" menu
//! item, route, download and panel embed are contributed in `web/mod.rs`.
use crate::{
    refresh::{validate_args, REFRESH_SERVICE, WORKER_ID},
    MODULE,
};
use bilibi_authz::{
    CapabilityDefinition as C, RoleDefinition, SystemPrincipalDefinition, VocabularyContribution,
    VOCABULARY,
};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_queue::{HandlerBinding, Worker, HANDLERS, WORKERS};

/// Every attachments read.
pub const VIEW: &str = "finance.attachments.view";
/// Attaches and replaces a source document.
pub const CREATE: &str = "finance.attachments.create";
/// Removes a draft record's attachment.
pub const DELETE: &str = "finance.attachments.delete";
/// Lengthens retention and places or releases a legal hold.
pub const MANAGE_RETENTION: &str = "finance.attachments.retention.manage";

pub const CAPABILITIES: [&str; 4] = [VIEW, CREATE, DELETE, MANAGE_RETENTION];

/// The system principal the finalisation refresh of a record's retention runs
/// as (Bilimbi `finance.attachments.retention_refresh`). Bilibi principal
/// names are one lowercase hyphenated segment, so the dots and the
/// underscore become hyphens.
pub const REFRESH_PRINCIPAL: &str = "finance-attachments-retention-refresh";

macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}

// The "finance" domain is the ledger's; "view", "create", "delete" and
// "manage" are Base Authz's verbs. This module declares no verb of its own, so
// it cannot clash with a verb another Domain declares.
inventory::submit! { Contribution::new(VOCABULARY, "finance/attachments", provenance!(), &VocabularyContribution {
    domains: &[],
    verbs: &[],
    capabilities: &[C::tenant(VIEW), C::tenant(CREATE), C::tenant(DELETE), C::tenant(MANAGE_RETENTION)],
    roles: &[RoleDefinition {
        code: "tenant_owner", name: "Tenant Owner", description: "Contributed system role; reconciled explicitly",
        grant_all: false, capabilities: &CAPABILITIES,
    }],
    system_principals: &[SystemPrincipalDefinition { name: REFRESH_PRINCIPAL, capabilities: &[MANAGE_RETENTION] }],
}) }

static REFRESH_WORKER: Worker = Worker {
    max_attempts: 10,
    validate: validate_args,
};
inventory::submit! { Contribution::new(WORKERS, WORKER_ID, provenance!(), &REFRESH_WORKER) }
inventory::submit! { Contribution::new(HANDLERS, WORKER_ID, provenance!(), &HandlerBinding {
    service: REFRESH_SERVICE,
    capability: MANAGE_RETENTION,
}) }
