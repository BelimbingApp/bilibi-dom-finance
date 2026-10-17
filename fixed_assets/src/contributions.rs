//! Bilimbi.Finance.FixedAssets.Contributions. Pages and menu land in NX3.
use crate::MODULE;
use bilibi_authz::{CapabilityDefinition as C, RoleDefinition, VocabularyContribution, VOCABULARY};
use bilibi_module_registry::{Contribution, Provenance};
pub const VIEW: &str = "finance.fixed-assets.view";
pub const CONFIGURE: &str = "finance.fixed-assets.finance-asset-configure";
pub const ACQUIRE: &str = "finance.fixed-assets.finance-asset-acquire";
pub const CAPABILITIES: [&str; 7] = [
    VIEW,
    CONFIGURE,
    ACQUIRE,
    "finance.fixed-assets.finance-asset-depreciate",
    "finance.fixed-assets.finance-asset-reverse",
    "finance.fixed-assets.finance-asset-dispose",
    "finance.fixed-assets.finance-asset-void",
];
inventory::submit! { Contribution::new(VOCABULARY,MODULE,Provenance {module:MODULE,source:file!(),line:line!()},&VocabularyContribution {
    domains:&[],verbs:&["finance-asset-configure", "finance-asset-acquire", "finance-asset-depreciate", "finance-asset-reverse", "finance-asset-dispose", "finance-asset-void"],
    capabilities:&[C::tenant(VIEW),C::tenant(CONFIGURE),C::tenant(ACQUIRE),C::tenant(CAPABILITIES[3]),C::tenant(CAPABILITIES[4]),C::tenant(CAPABILITIES[5]),C::tenant(CAPABILITIES[6])],
    roles:&[RoleDefinition {code:"tenant_owner",name:"Tenant Owner",description:"Contributed system role; reconciled explicitly",grant_all:false,capabilities:&CAPABILITIES}],system_principals:&[],
}) }
