//! Bilimbi.Finance.Sst.Contributions (routes and menu live in `web`).
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};
pub const VIEW: &str = "finance.sst.view";
pub const MANAGE_SETTINGS: &str = "finance.sst.settings.manage";
pub const PREPARE_RETURNS: &str = "finance.sst.returns.sst-prepare";
pub const FINALISE_RETURNS: &str = "finance.sst.returns.sst-finalise";
pub const AMEND_RETURNS: &str = "finance.sst.returns.sst-amend";
pub const CAPABILITIES: [&str; 5] = [
    VIEW,
    MANAGE_SETTINGS,
    PREPARE_RETURNS,
    FINALISE_RETURNS,
    AMEND_RETURNS,
];
// The "finance" domain is finance/ledger's and "view" and "manage" are the
// platform's; this module's own verbs carry the `sst-` prefix.
inventory::submit! { Contribution::new(VOCABULARY,MODULE,Provenance {module:MODULE,source:file!(),line:line!()},&VocabularyContribution {
    domains:&[],verbs:&["sst-prepare", "sst-finalise", "sst-amend"],
    capabilities:&[C::tenant(VIEW),C::tenant(MANAGE_SETTINGS),C::tenant(PREPARE_RETURNS),C::tenant(FINALISE_RETURNS),C::tenant(AMEND_RETURNS)],
    roles:&[RoleDefinition {code:"tenant_owner",name:"Tenant Owner",description:"Contributed system role; reconciled explicitly",grant_all:false,capabilities:&CAPABILITIES}],system_principals:&[],
}) }
