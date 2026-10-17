//! The module's one setting (Bilimbi `Bilimbi.Finance.Reports.Contributions`,
//! `settings`): `finance.reports.sections`, the sections that group accounts in
//! the reports, at tenant or company scope. It is edited on the operator
//! Settings screen by holders of the ledger's `finance.ledger.accounts.manage`,
//! because arranging the reports is part of keeping the chart of accounts. No
//! heading or account code is a default: the layout is the company's data.
use crate::contributions::provenance;
use crate::{layout, MODULE};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
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
    editor: Some(Editor {
        group: "Reports",
        label: "Report sections",
        help: "Sections that group accounts in the reports, in order: a JSON list of \
               {\"report\": \"profit_and_loss\", \"balance_sheet\" or \"cash_position\", \
               \"name\": heading, \"accounts\": [account codes]}. A listed account brings its \
               sub-accounts; the cash position lists only the accounts its sections name.",
        capability: Some("finance.ledger.accounts.manage"),
        reveal_duration_ms: 5_000,
    }),
})) }
