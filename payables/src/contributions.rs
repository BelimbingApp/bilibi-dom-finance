//! Bilimbi.Finance.Payables.Contributions. Pages/menu land in NP4.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};
pub const VIEW: &str = "finance.payables.view";
pub const MANAGE_SETTINGS: &str = "finance.payables.settings.manage";
pub const CREATE_DOCUMENTS: &str = "finance.payables.documents.create";
pub const APPROVE_DOCUMENTS: &str = "finance.payables.documents.approve";
pub const POST_DOCUMENTS: &str = "finance.payables.documents.post";
pub const SUBMIT_PAYMENTS: &str = "finance.payables.payments.submit";
pub const POST_PAYMENTS: &str = "finance.payables.payments.post";
pub const CAPABILITIES: [&str; 8] = [
    VIEW,
    MANAGE_SETTINGS,
    CREATE_DOCUMENTS,
    APPROVE_DOCUMENTS,
    POST_DOCUMENTS,
    SUBMIT_PAYMENTS,
    POST_PAYMENTS,
    "finance.payables.match.approve",
];
inventory::submit! { Contribution::new(VOCABULARY,MODULE,Provenance {module:MODULE,source:file!(),line:line!()},&VocabularyContribution {
    domains:&[],verbs:&[],
    capabilities:&[C::tenant(VIEW),C::tenant(MANAGE_SETTINGS),C::tenant(CREATE_DOCUMENTS),C::tenant(APPROVE_DOCUMENTS),C::tenant(POST_DOCUMENTS),C::tenant("finance.payables.payments.submit"),C::tenant("finance.payables.payments.post"),C::tenant("finance.payables.match.approve")],
    roles:&[RoleDefinition {code:"tenant_owner",name:"Tenant Owner",description:"Contributed system role; reconciled explicitly",grant_all:false,capabilities:&CAPABILITIES}],system_principals:&[],
}) }
