//! A company's e-invoicing settings (Bilimbi
//! `Bilimbi.Finance.Einvoice.Configuration`, plus the platform URL settings of
//! its `MyInvois.Client` and `MyInvois`): the environment it submits to, the
//! format version it submits in, the credentials the authority issued it for
//! each environment, its signing certificate and private key, the supplier
//! profile every document it issues carries, and the ledger accounts a
//! validated document posts to.
//!
//! Every company value is a company-scoped Base Setting in the editor group
//! `E-invoice`, guarded by `finance.einvoice.settings.manage`, so the
//! company's own operators set them on the Settings screen. The client
//! secrets, certificate and private key are encrypted settings: masked on
//! screen and revealable only through Base Settings' password-checked reveal.
//! Nothing here reads a secret back: the facade asks Base Settings for the
//! masked value and learns only whether one is set.
//!
//! Each company registers with the authority and holds its own certificate; no
//! value has a default that names a country, a tax number or a credential. The
//! authority's API and portal base URLs are platform (global) settings an
//! operator changes only when the authority publishes a new host.
use crate::authority::FormatKind;
use crate::certificate;
use crate::contributions::{provenance, SETTINGS};
use crate::{myinvois, MODULE};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_settings::{Definition, Editor, ScopeKind, Setting, Validator, ValueType, DEFINITIONS};
use serde_json::{json, Value};

/// The editor group the company settings appear under.
const GROUP: &str = "E-invoice";
/// The capability that edits the platform URL settings.
const OPERATOR: &str = "base.settings.global.manage";
const COMPANY: &[ScopeKind] = &[ScopeKind::Company];
const GLOBAL: &[ScopeKind] = &[ScopeKind::Global];

/// The environments a company can submit to.
pub(crate) const ENVIRONMENTS: [&str; 2] = ["sandbox", "production"];

/// A nullable setting's validators see `null` too (a cleared value); only a
/// string is checked.
fn optional(value: &Value, accepts: impl Fn(&str) -> bool) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => accepts(text),
        _ => false,
    }
}

/// Between 1 and `max` characters once trimmed.
fn text_within(text: &str, max: usize) -> bool {
    (1..=max).contains(&text.trim().chars().count())
}

fn valid_date(text: &str) -> bool {
    text.parse::<jiff::civil::Date>().is_ok()
}

macro_rules! define {
    (
        $key:expr, $scopes:expr, $type:expr, $default:expr,
        nullable: $nullable:expr, encrypted: $encrypted:expr, bounds: ($min:expr, $max:expr),
        validator: $validator:expr,
        editor: ($group:expr, $label:expr, $help:expr, $capability:expr)
    ) => {
        inventory::submit! { Contribution::new(DEFINITIONS, $key, provenance!(), &Setting::Definition(Definition {
            value_type: $type, scopes: $scopes, default: $default,
            nullable: $nullable, encrypted: $encrypted, minimum: $min, maximum: $max,
            validator: $validator,
            editor: Some(Editor { group: $group, label: $label, help: $help, capability: Some($capability), reveal_duration_ms: 5_000 }),
        })) }
    };
}

// A company text value that may be left blank.
macro_rules! company_text {
    ($key:expr, $label:expr, $help:expr, $validator:expr) => {
        define!($key, COMPANY, ValueType::String, || Value::Null,
            nullable: true, encrypted: false, bounds: (None, None), validator: $validator,
            editor: (GROUP, $label, $help, SETTINGS));
    };
}

// A company secret: encrypted, blank until set.
macro_rules! company_secret {
    ($key:expr, $label:expr, $help:expr, $validator:expr) => {
        define!($key, COMPANY, ValueType::String, || Value::Null,
            nullable: true, encrypted: true, bounds: (None, None), validator: $validator,
            editor: (GROUP, $label, $help, SETTINGS));
    };
}

// A supplier-profile field limited to `$max` characters.
macro_rules! supplier_text {
    ($field:literal, $label:expr, $help:expr, $max:literal) => {
        company_text!(
            concat!("finance.einvoice.supplier.", $field),
            $label,
            $help,
            Some(Validator {
                accepts: |value| optional(value, |text| text_within(text, $max)),
                message: concat!("must be at most ", $max, " characters"),
            })
        );
    };
}

// A supplier-profile field whose shape the authority fixes.
macro_rules! supplier_format {
    ($field:literal, $label:expr, $help:expr, $kind:expr, $message:expr) => {
        company_text!(
            concat!("finance.einvoice.supplier.", $field),
            $label,
            $help,
            Some(Validator {
                accepts: |value| optional(value, |text| myinvois::valid_format($kind, text)),
                message: $message,
            })
        );
    };
}

// A posting account code of the company's own chart: none is a default.
macro_rules! posting_account {
    ($field:literal, $label:expr, $help:expr) => {
        company_text!(
            concat!("finance.einvoice.posting.", $field),
            $label,
            $help,
            Some(Validator {
                accepts: |value| optional(value, |text| text_within(text, 50)),
                message: "must be at most 50 characters",
            })
        );
    };
}

macro_rules! environment_settings {
    ($environment:literal, $title:literal) => {
        company_text!(
            concat!("finance.einvoice.", $environment, ".client_id"),
            concat!($title, " client ID"),
            concat!(
                "The client ID the authority issued for ",
                $environment,
                " ERP access."
            ),
            None
        );
        company_secret!(
            concat!("finance.einvoice.", $environment, ".client_secret"),
            concat!($title, " client secret"),
            concat!(
                "The client secret issued with the ",
                $environment,
                " client ID."
            ),
            None
        );
        company_text!(
            concat!("finance.einvoice.", $environment, ".secret_expires_on"),
            concat!($title, " secret expiry date"),
            concat!(
                "The date the ",
                $environment,
                " client secret expires, as YYYY-MM-DD."
            ),
            Some(Validator {
                accepts: |value| optional(value, valid_date),
                message: "must be a date as YYYY-MM-DD",
            })
        );
    };
}

// A platform URL: the host an operator changes only when the authority
// publishes a new one.
macro_rules! platform_url {
    ($key:expr, $default:expr, $label:expr, $help:expr) => {
        define!($key, GLOBAL, ValueType::String, || json!($default),
            nullable: false, encrypted: false, bounds: (None, None),
            validator: Some(Validator {
                accepts: |value| value.as_str().is_some_and(myinvois::valid_url),
                message: "must be an https URL without a path",
            }),
            editor: (GROUP, $label, $help, OPERATOR));
    };
}

// Connection.
define!("finance.einvoice.environment", COMPANY, ValueType::String, || json!("sandbox"),
    nullable: false, encrypted: false, bounds: (None, None),
    validator: Some(Validator {
        accepts: |value| value.as_str().is_some_and(|text| ENVIRONMENTS.contains(&text)),
        message: "must be sandbox or production",
    }),
    editor: (GROUP, "Environment",
        "Sandbox submits test documents; production submits documents the authority keeps.", SETTINGS));
company_secret!(
    "finance.einvoice.certificate_pem",
    "Signing certificate (PEM)",
    "The company's X.509 signing certificate in PEM form.",
    Some(Validator {
        accepts: |value| optional(value, certificate::valid_certificate_pem),
        message: "must be one PEM-encoded X.509 certificate",
    })
);
company_secret!(
    "finance.einvoice.private_key_pem",
    "Signing private key (PEM)",
    "The certificate's RSA private key in unencrypted PEM form.",
    Some(Validator {
        accepts: |value| optional(value, certificate::valid_private_key_pem),
        message: "must be one unencrypted PEM-encoded RSA private key",
    })
);
define!("finance.einvoice.expiry_alert_days", COMPANY, ValueType::Integer, || json!(30),
    nullable: false, encrypted: false, bounds: (Some(1.0), Some(365.0)), validator: None,
    editor: (GROUP, "Expiry alert (days)",
        "Warn this many days before the certificate or a client secret expires.", SETTINGS));
define!("finance.einvoice.document_version", COMPANY, ValueType::String, || json!(myinvois::VERSION),
    nullable: false, encrypted: false, bounds: (None, None),
    validator: Some(Validator {
        accepts: |value| value.as_str().is_some_and(myinvois::valid_document_version),
        message: "must be a version the authority accepts",
    }),
    editor: (GROUP, "Document format version",
        "The authority's format version documents are submitted in. A signed version needs the certificate and private key; an unsigned one is for sandbox testing before the certificate arrives.", SETTINGS));

// Credentials, per environment.
environment_settings!("sandbox", "Sandbox");
environment_settings!("production", "Production");

// The ledger accounts a validated document's entry names, by the account
// codes of the company's own chart.
posting_account!(
    "receivable_account",
    "Receivable account",
    "Account code debited by a validated invoice or debit note and credited by a credit note."
);
posting_account!(
    "revenue_account",
    "Revenue account",
    "Account code credited with the tax-exclusive amount of a validated invoice or debit note."
);
posting_account!(
    "tax_account",
    "Tax account",
    "Account code credited with the tax of a validated invoice or debit note."
);
posting_account!("settlement_account", "Settlement account",
    "Account code credited by a validated refund note, in place of the receivable account, and debited by a self-billed refund note.");
posting_account!("payable_account", "Payable account",
    "Account code credited by a validated self-billed invoice or debit note and debited by a self-billed credit note.");
posting_account!("expense_account", "Expense account",
    "Account code debited with the tax-exclusive amount of a validated self-billed invoice or debit note.");
posting_account!(
    "purchase_tax_account",
    "Purchase tax account",
    "Account code debited with the tax of a validated self-billed invoice or debit note."
);

// The supplier profile every document the company issues carries.
supplier_text!(
    "name",
    "Supplier name",
    "The registered name on documents. Blank uses the company's name.",
    300
);
supplier_format!(
    "tax_id",
    "Supplier TIN",
    "The tax identification number the authority registered the company under.",
    FormatKind::TaxId,
    myinvois::TAX_ID_MESSAGE
);
supplier_format!("registration_type", "Supplier registration type",
    "The kind of registration number: BRN for a business, NRIC, PASSPORT, or ARMY for an individual. Blank means BRN.",
    FormatKind::RegistrationType, myinvois::REGISTRATION_TYPE_MESSAGE);
supplier_text!("registration_number", "Supplier registration number",
    "The business registration number, as on the signing certificate, or the individual's identification number.", 20);
supplier_text!(
    "sales_tax_registration",
    "Sales and service tax registration",
    "Up to two registration numbers separated by a semicolon. Blank when not registered.",
    35
);
supplier_text!(
    "tourism_tax_registration",
    "Tourism tax registration",
    "Blank when not registered.",
    17
);
supplier_format!(
    "industry_code",
    "Industry classification code",
    "The five-digit industry code the company registered.",
    FormatKind::IndustryCode,
    myinvois::INDUSTRY_CODE_MESSAGE
);
supplier_text!(
    "business_activity",
    "Business activity",
    "The description of the business activity that goes with the code.",
    300
);
supplier_format!(
    "phone",
    "Supplier phone",
    "In international form, for example +60123456789.",
    FormatKind::Phone,
    myinvois::PHONE_MESSAGE
);
supplier_text!(
    "email",
    "Supplier email",
    "Optional contact email on documents.",
    320
);
supplier_text!(
    "address_line1",
    "Address line 1",
    "First line of the supplier address.",
    150
);
supplier_text!(
    "address_line2",
    "Address line 2",
    "Optional second line.",
    150
);
supplier_text!(
    "address_line3",
    "Address line 3",
    "Optional third line.",
    150
);
supplier_text!("city", "City", "City of the supplier address.", 50);
supplier_text!(
    "postcode",
    "Postcode",
    "Postcode of the supplier address.",
    50
);
supplier_format!(
    "state_code",
    "State code",
    "The authority's two-digit code for the state of the supplier address.",
    FormatKind::StateCode,
    myinvois::STATE_CODE_MESSAGE
);
supplier_format!(
    "country_code",
    "Country code",
    "Three-letter ISO 3166 code of the supplier address.",
    FormatKind::CountryCode,
    myinvois::COUNTRY_CODE_MESSAGE
);

// The authority's hosts, per environment: platform settings.
// The sandbox API may also be a stand-in on this machine's loopback, so a
// company's clearance can be exercised without the authority's sandbox.
define!("finance.einvoice.sandbox.api_url", GLOBAL, ValueType::String,
    || json!("https://preprod-api.myinvois.hasil.gov.my"),
    nullable: false, encrypted: false, bounds: (None, None),
    validator: Some(Validator {
        accepts: |value| value.as_str().is_some_and(myinvois::valid_sandbox_api_url),
        message: "must be an https URL without a path",
    }),
    editor: (GROUP, "E-invoice sandbox API URL",
        "Base URL of the MyInvois sandbox API. Change it only when the authority publishes a new host.",
        OPERATOR));
platform_url!("finance.einvoice.production.api_url", "https://api.myinvois.hasil.gov.my",
    "E-invoice production API URL",
    "Base URL of the MyInvois production API. Change it only when the authority publishes a new host.");
platform_url!(
    "finance.einvoice.sandbox.portal_url",
    "https://preprod.myinvois.hasil.gov.my",
    "E-invoice sandbox portal URL",
    "Base URL of the MyInvois sandbox portal that a document's validation link opens."
);
platform_url!(
    "finance.einvoice.production.portal_url",
    "https://myinvois.hasil.gov.my",
    "E-invoice production portal URL",
    "Base URL of the MyInvois production portal that a document's validation link opens."
);

/// The setting key of a supplier-profile field.
pub(crate) fn supplier_key(field: &str) -> String {
    format!("finance.einvoice.supplier.{field}")
}

/// The supplier-profile fields, in the order the profile lists them.
pub(crate) const SUPPLIER_FIELDS: [&str; 17] = [
    "name",
    "tax_id",
    "registration_type",
    "registration_number",
    "sales_tax_registration",
    "tourism_tax_registration",
    "industry_code",
    "business_activity",
    "phone",
    "email",
    "address_line1",
    "address_line2",
    "address_line3",
    "city",
    "postcode",
    "state_code",
    "country_code",
];
