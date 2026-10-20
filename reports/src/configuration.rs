//! The module's one setting (Bilimbi `Bilimbi.Finance.Reports.Contributions`,
//! `settings`): `finance.reports.sections`, the sections that group accounts in
//! the reports, at tenant or company scope. The Report sections form edits a
//! company through the sections contracts, with `finance.ledger.accounts.manage`.
//! Stored headings and account codes are the company's data.
use crate::contributions::provenance;
use crate::{layout, MODULE};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_settings::{Definition, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::Value;

/// The setting that holds a company's report sections.
pub const SECTIONS: &str = "finance.reports.sections";

inventory::submit! { Contribution::new(DEFINITIONS, SECTIONS, provenance!(), &Setting::Definition(Definition {
    value_type: ValueType::Array, scopes: &[ScopeKind::Tenant, ScopeKind::Company],
    default: || Value::Array(Vec::new()),
    nullable: false, encrypted: false, minimum: None, maximum: None,
    validator: Some(Validator {
        accepts: layout::valid,
        message: "must be a list of sections, each with only a report, a name and account codes",
    }),
    // The company form calls the same reviewed contract as agents.
    editor: None,
})) }
