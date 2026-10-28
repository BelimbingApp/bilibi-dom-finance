//! Ledger setup proposals, edited through Base Settings or the ledger's
//! reviewed operation. A proposal never creates accounts.
use crate::{MANAGE_TEMPLATES, MODULE};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::Value;

pub(crate) const DEFAULT_TEMPLATE: &str = "finance.ledger.default_chart_template";

macro_rules! define {
    ($key:expr, $label:expr, $help:expr) => {
        inventory::submit! { Contribution::new(DEFINITIONS, $key,
            Provenance { module: MODULE, source: file!(), line: line!() },
            &Setting::Definition(Definition {
                value_type: ValueType::String, scopes: &[ScopeKind::Global, ScopeKind::Tenant],
                default: || Value::Null, nullable: true, encrypted: false,
                minimum: None, maximum: None,
                validator: Some(Validator {
                    accepts: |value| value.is_null() || value.as_str().is_some_and(|key| !key.is_empty() && key.len() <= 100),
                    message: "Choose a starter template key, or clear the proposal",
                }),
                editor: Some(Editor { group: "Ledger", label: $label, help: $help,
                    capability: Some(MANAGE_TEMPLATES), reveal_duration_ms: 5_000 }),
            })) }
    };
}
define!(
    DEFAULT_TEMPLATE,
    "Proposed starter chart",
    "A starter template key proposed for new books. Set a bundled key globally, or a bundled or saved key for this tenant. Application requires a separate preview and confirmation. Clear to propose none."
);
