//! E-invoicing, module `finance/einvoice` (Bilimbi
//! `Bilimbi.Finance.Einvoice`): the fiscal documents a company issues through
//! a clearance authority. This version holds the schema, the company's setup
//! (settings, credentials, signing certificate and supplier profile, read here
//! without any secret), the [`Authority`] contract with the one authority this
//! release clears documents with (MyInvois), and the authority's code lists
//! with each company's overrides. Issuing, signing, submitting and posting
//! documents follow in later versions of this module. Other modules never read
//! its tables.
//!
//! Every function takes the sealed [`Scope`] it was handed and an explicit
//! company id. Reads authorize their capability and writes theirs for the
//! scope's actor and the target company at the moment they run, through Core
//! Company's `authorize_target`: a missing grant, or a sibling company without
//! tenant-wide reach, is [`EinvoiceError::Unauthorized`], and a company the
//! scope cannot see is [`EinvoiceError::CompanyNotFound`]. A write then holds
//! the company's row lock inside an audited mutation transaction, so an
//! archived company is [`EinvoiceError::CompanyArchived`].
mod authority;
mod certificate;
mod code_lists;
mod configuration;
mod contributions;
mod myinvois;

pub use authority::{
    Authority, CertificateIdentity, CertificateSubject, ConsolidationWindow, DocumentVersion,
    FormatKind, PostingAccount, PostingRule, Side, Supplier, SupplierField,
};
pub use code_lists::Codes;
pub use contributions::{CANCEL, CAPABILITIES, ISSUE, SETTINGS, SUBMIT, VIEW};

use bilibi_company::{organisation, WriteError};
use bilibi_database::{Change, Db, Error, MutationBoundary, MutationTransaction};
use bilibi_module_registry::{Layer, Module, Service};
use bilibi_party_ids::OrganisationId;
use bilibi_settings::{Resolved, Settings, Target};
use bilibi_tenancy::Scope;
use jiff::civil::Date;
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tokio_postgres::GenericClient;

/// The descriptor id.
pub const MODULE: &str = "finance/einvoice";
/// The registry service id of the [`Einvoice`] facade.
pub const SERVICE: &str = "finance/einvoice.einvoice";

inventory::submit! {
    Module {
        id: MODULE, layer: Layer::Domains,
        // TODO(finance/ledger edge): the ledger bridge (NE4) adds the edge to
        // `finance/ledger` where Bilimbi has it, with the first call into it.
        dependencies: &["base/authz", "base/database", "base/party_ids", "base/settings", "base/tenancy", "core/company"],
        migrations: &[
            ("finance-einvoice-001-documents", include_str!("../migrations/001-documents.sql")),
            ("finance-einvoice-002-consolidation", include_str!("../migrations/002-consolidation.sql")),
            ("finance-einvoice-003-source-links", include_str!("../migrations/003-source-links.sql")),
        ],
        services: &[Service { id: SERVICE, factory: |services| Ok(Arc::new(Einvoice {
            db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
            mutations: services.get("base/database.mutations")?,
            settings: services.get("base/settings.store")?,
        })) }],
    }
}

pub type Result<T> = std::result::Result<T, EinvoiceError>;

/// Why a call did not happen.
#[derive(Debug, thiserror::Error)]
pub enum EinvoiceError {
    /// A system scope, a missing grant, or a sibling company without
    /// tenant-wide reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company the scope cannot see: missing, deleted or another tenant's.
    #[error("Not found")]
    CompanyNotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    /// A list the authority does not publish.
    #[error("Unknown code list")]
    UnknownList,
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(#[from] Error),
}

/// One refusal of a submitted value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}
impl FieldError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// A company's override of one code of the authority's list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CodeOverride {
    pub list: String,
    pub code: String,
    pub description: Option<String>,
    pub active: bool,
}
/// What changes on a code override: `description: Some(Some(text))` renames
/// the code (or names one the shipped list lacks), `Some(None)` removes the
/// company's name for it, `active: Some(false)` retires it. A field left
/// `None` is kept; a new override is active unless said otherwise.
#[derive(Clone, Debug, Default)]
pub struct CodeOverrideChange {
    pub description: Option<Option<String>>,
    pub active: Option<bool>,
}

/// The ledger account codes a validated document posts to, by the company's
/// own chart; a blank one is `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PostingAccounts {
    pub receivable_account: Option<String>,
    pub revenue_account: Option<String>,
    pub tax_account: Option<String>,
    pub settlement_account: Option<String>,
    pub payable_account: Option<String>,
    pub expense_account: Option<String>,
    pub purchase_tax_account: Option<String>,
}
impl PostingAccounts {
    /// The company's account code for `account`.
    pub fn get(&self, account: PostingAccount) -> Option<&str> {
        match account {
            PostingAccount::Receivable => &self.receivable_account,
            PostingAccount::Revenue => &self.revenue_account,
            PostingAccount::Tax => &self.tax_account,
            PostingAccount::Settlement => &self.settlement_account,
            PostingAccount::Payable => &self.payable_account,
            PostingAccount::Expense => &self.expense_account,
            PostingAccount::PurchaseTax => &self.purchase_tax_account,
        }
        .as_deref()
    }
}

/// What a company has configured, without any secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Setup {
    /// `sandbox` or `production`.
    pub environment: String,
    /// The authority's format version documents are submitted in.
    pub document_version: String,
    pub posting_accounts: PostingAccounts,
    /// Whether the current environment has a client ID.
    pub client_id_set: bool,
    /// Whether the current environment has a client secret.
    pub client_secret_set: bool,
    /// When the current environment's client secret expires, as `YYYY-MM-DD`.
    pub client_secret_expires_on: Option<String>,
    /// Whole days from today (the database's UTC date) until it expires;
    /// negative once expired.
    pub client_secret_days_left: Option<i64>,
    pub certificate_set: bool,
    pub private_key_set: bool,
    /// Warn this many days before the certificate or a client secret expires.
    pub expiry_alert_days: i64,
    /// The supplier profile, with the company's name for a blank `name` and
    /// `BRN` for a blank registration type.
    pub supplier: Supplier,
    /// The supplier fields every document carries that the profile leaves blank.
    pub supplier_missing: Vec<&'static str>,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Einvoice {
    db: Db,
    mutations: Arc<MutationBoundary>,
    settings: Arc<Settings>,
}

impl Einvoice {
    /// The authority every company's documents are cleared with.
    pub fn authority() -> impl Authority {
        myinvois::MyInvois
    }

    /// The lists the authority publishes, sorted.
    pub fn code_lists() -> Vec<&'static str> {
        code_lists::lists(myinvois::ID)
    }

    /// The company's active codes in one of the authority's lists, code to
    /// description (`finance.einvoice.view`): the shipped list with the
    /// company's overrides applied. `UnknownList` for a list the authority
    /// does not publish.
    pub async fn list_codes(&self, scope: &Scope, company_id: i64, list: &str) -> Result<Codes> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        let authority = Self::authority().id();
        if !code_lists::lists(authority).contains(&list) {
            return Err(EinvoiceError::UnknownList);
        }
        Ok(code_lists::codes(&**client, scope.tenant(), company_id, authority, list).await?)
    }

    /// Overrides one code for the company (`finance.einvoice.settings.manage`):
    /// `description` renames it, or names a code the shipped list lacks, and
    /// `active: Some(false)` retires it. `UnknownList` for a list the
    /// authority does not publish; refused values are `Invalid`. The audit
    /// action is `finance.einvoice.code.overridden`.
    pub async fn override_code(
        &self,
        scope: &Scope,
        company_id: i64,
        list: &str,
        code: &str,
        change: &CodeOverrideChange,
    ) -> Result<CodeOverride> {
        let authority = Self::authority().id();
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                SETTINGS,
                "finance.einvoice.code.overridden",
            )
            .await?;
        if !code_lists::lists(authority).contains(&list) {
            return Err(EinvoiceError::UnknownList);
        }
        let tenant = scope.tenant();
        let valid =
            code_lists::validate(tx.sql(), tenant, company_id, authority, list, code, change)
                .await?
                .map_err(EinvoiceError::Invalid)?;
        let next = valid.next.clone();
        if valid.current.as_ref() != Some(&valid.next) {
            let current = valid.current;
            tx.mutate(move |sql| {
                Box::pin(async move {
                    let id = code_lists::upsert(sql, tenant, company_id, authority, &next).await?;
                    let change = Change {
                        kind: "finance.einvoice.code_override".into(),
                        id,
                        old: current.as_ref().map_or(Value::Null, code_lists::to_json),
                        new: code_lists::to_json(&next),
                    };
                    Ok(((), vec![change]))
                })
            })
            .await?;
        }
        tx.commit().await?;
        Ok(valid.next)
    }

    /// What the company has configured, never a secret
    /// (`finance.einvoice.settings.manage`): the environment, which
    /// credentials are present, the client secret's expiry, the supplier
    /// profile and what it still lacks.
    pub async fn setup(&self, scope: &Scope, company_id: i64) -> Result<Setup> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, SETTINGS).await?;
        let name = organisation::identity(&**client, scope, company_id)
            .await?
            .ok_or(EinvoiceError::CompanyNotFound)?
            .display_name;
        let target = Target::Company(
            OrganisationId::new(company_id).map_err(|_| EinvoiceError::CompanyNotFound)?,
        );
        let environment = text(
            &self
                .settings
                .get(&**client, scope, target, "finance.einvoice.environment")
                .await?
                .value,
        )
        .unwrap_or_else(|| "sandbox".into());
        let prefix = format!("finance.einvoice.{environment}");
        let (client_id, client_secret, expires) = (
            format!("{prefix}.client_id"),
            format!("{prefix}.client_secret"),
            format!("{prefix}.secret_expires_on"),
        );
        let posting: Vec<String> = PostingAccount::ALL
            .iter()
            .map(|account| format!("finance.einvoice.posting.{}", account.field()))
            .collect();
        let supplier: Vec<String> = configuration::SUPPLIER_FIELDS
            .iter()
            .map(|field| configuration::supplier_key(field))
            .collect();
        let mut keys: Vec<&str> = vec![
            "finance.einvoice.document_version",
            "finance.einvoice.expiry_alert_days",
            &client_id,
            &expires,
        ];
        keys.extend(posting.iter().map(String::as_str));
        keys.extend(supplier.iter().map(String::as_str));
        let values = self
            .settings
            .resolve_many(&**client, scope, target, &keys)
            .await?;
        // Secrets are read masked: a stored one comes back `null` with its
        // source, which is all the setup needs to know.
        let masked = self
            .settings
            .resolve_masked(
                &**client,
                scope,
                target,
                &[
                    &client_secret,
                    "finance.einvoice.certificate_pem",
                    "finance.einvoice.private_key_pem",
                ],
            )
            .await?;
        let value = |key: &str| values.get(key).and_then(|resolved| text(&resolved.value));
        let is_set = |key: &str| {
            masked
                .get(key)
                .is_some_and(|r: &Resolved| r.source.is_some())
        };

        let expires_on = value(&expires).filter(|date| date.parse::<Date>().is_ok());
        let days_left = match &expires_on {
            Some(date) => Some(days_until(&**client, date).await?),
            None => None,
        };
        let mut profile = Supplier {
            name: value(&configuration::supplier_key("name")),
            tax_id: value(&configuration::supplier_key("tax_id")),
            registration_type: value(&configuration::supplier_key("registration_type")),
            registration_number: value(&configuration::supplier_key("registration_number")),
            sales_tax_registration: value(&configuration::supplier_key("sales_tax_registration")),
            tourism_tax_registration: value(&configuration::supplier_key(
                "tourism_tax_registration",
            )),
            industry_code: value(&configuration::supplier_key("industry_code")),
            business_activity: value(&configuration::supplier_key("business_activity")),
            phone: value(&configuration::supplier_key("phone")),
            email: value(&configuration::supplier_key("email")),
            address_line1: value(&configuration::supplier_key("address_line1")),
            address_line2: value(&configuration::supplier_key("address_line2")),
            address_line3: value(&configuration::supplier_key("address_line3")),
            city: value(&configuration::supplier_key("city")),
            postcode: value(&configuration::supplier_key("postcode")),
            state_code: value(&configuration::supplier_key("state_code")),
            country_code: value(&configuration::supplier_key("country_code")),
        };
        profile.name.get_or_insert(name);
        profile
            .registration_type
            .get_or_insert_with(|| "BRN".into());
        let posting_value = |account: PostingAccount| {
            value(&format!("finance.einvoice.posting.{}", account.field()))
        };
        Ok(Setup {
            environment,
            document_version: value("finance.einvoice.document_version")
                .unwrap_or_else(|| myinvois::VERSION.into()),
            posting_accounts: PostingAccounts {
                receivable_account: posting_value(PostingAccount::Receivable),
                revenue_account: posting_value(PostingAccount::Revenue),
                tax_account: posting_value(PostingAccount::Tax),
                settlement_account: posting_value(PostingAccount::Settlement),
                payable_account: posting_value(PostingAccount::Payable),
                expense_account: posting_value(PostingAccount::Expense),
                purchase_tax_account: posting_value(PostingAccount::PurchaseTax),
            },
            client_id_set: value(&client_id).is_some(),
            client_secret_set: is_set(&client_secret),
            client_secret_expires_on: expires_on,
            client_secret_days_left: days_left,
            certificate_set: is_set("finance.einvoice.certificate_pem"),
            private_key_set: is_set("finance.einvoice.private_key_pem"),
            expiry_alert_days: values
                .get("finance.einvoice.expiry_alert_days")
                .and_then(|resolved| resolved.value.as_i64())
                .unwrap_or(30),
            supplier_missing: Self::authority()
                .supplier_missing(&profile)
                .into_iter()
                .map(SupplierField::field)
                .collect(),
            supplier: profile,
        })
    }

    /// Every write: the capability and company reach now, then the audited
    /// transaction holding the company's row lock.
    async fn write<'c>(
        &self,
        client: &'c mut tokio_postgres::Client,
        scope: &Scope,
        company_id: i64,
        capability: &str,
        action: &str,
    ) -> Result<MutationTransaction<'c>> {
        authorize(&*client, scope, company_id, capability).await?;
        let tx = self
            .mutations
            .begin(client, scope.mutation_context(action))
            .await?;
        organisation::require_writable(tx.sql(), scope, company_id, true)
            .await
            .map_err(|error| match error {
                WriteError::NotFound => EinvoiceError::CompanyNotFound,
                WriteError::Archived => EinvoiceError::CompanyArchived,
                WriteError::Access(error) => refusal(error),
                WriteError::InUse | WriteError::Stale | WriteError::Invalid(_) => {
                    EinvoiceError::Failed(Error::Internal)
                }
            })?;
        Ok(tx)
    }
}

/// A trimmed, non-blank string value.
fn text(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Whole days from the database's UTC date until `date` (`YYYY-MM-DD`).
async fn days_until(client: &impl GenericClient, date: &str) -> Result<i64> {
    let row = client
        .query_one(
            "SELECT ($1::text::date - (now() AT TIME ZONE 'UTC')::date)::bigint",
            &[&date],
        )
        .await
        .map_err(Error::from)?;
    Ok(row.get(0))
}

/// The actor's current grant and company reach decide every call.
async fn authorize(
    client: &impl GenericClient,
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<()> {
    organisation::authorize_target(client, scope, company_id, capability)
        .await
        .map(|_| ())
        .map_err(refusal)
}

fn refusal(error: Error) -> EinvoiceError {
    match error {
        Error::Forbidden | Error::Unauthorized => EinvoiceError::Unauthorized,
        Error::NotFound => EinvoiceError::CompanyNotFound,
        other => EinvoiceError::Failed(other),
    }
}
