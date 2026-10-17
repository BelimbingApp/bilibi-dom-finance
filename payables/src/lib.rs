//! Company payables documents, mirroring Bilimbi.Finance.Payables.
//! Records are immutable after creation; approval and posting advance their
//! status, and corrections are debit notes. SQL and validation remain private.
mod agent_operations;
mod contributions;
mod documents;
mod payments;
pub use payments::{
    Allocation, AllocationInput, NewPayment, Payment, PaymentDecision, PaymentQuery,
    PaymentRequest, PaymentRequestQuery,
};
mod balances;
pub use balances::{
    AgedDocument, AgedParty, AgedPayables, AgingQuery, ControlAccount, OtherPosting, TaxQuery,
    TaxTotal, TieOut,
};
mod evidence;
pub use evidence::EvidencePage;
mod settings;
mod source;
mod store;

use bilibi_agent_api::OperationClient;
use bilibi_company::organisation;
use bilibi_database::{Db, Error, MutationTransaction};
use bilibi_finance_ledger::{FieldError, Ledger, LedgerError, Money};
use bilibi_module_registry::{Layer, Module, Service};
use bilibi_tenancy::Scope;
pub use contributions::{
    APPROVE_DOCUMENTS, CAPABILITIES, CREATE_DOCUMENTS, MANAGE_SETTINGS, POST_DOCUMENTS,
    POST_PAYMENTS, SUBMIT_PAYMENTS, VIEW,
};
use jiff::civil::Date;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_postgres::GenericClient;

pub const MODULE: &str = "finance/payables";
pub const SERVICE: &str = "finance/payables.payables";
pub const PAYMENT_SOURCE: &str = "finance/payables/payment";
pub const DOCUMENT_SOURCE: &str = "finance/payables/document";

inventory::submit! { Module {
    id: MODULE, layer: Layer::Domains,
    dependencies: &["base/agent_api", "base/authz", "base/database", "base/party_ids", "base/tenancy", "core/company", "finance/attachments", "finance/ledger"],
    migrations: &[("finance-payables-001-documents", include_str!("../migrations/001-documents.sql")), ("finance-payables-002-payments", include_str!("../migrations/002-payments.sql"))],
    services: &[Service { id: SERVICE, factory: |s| { let payables = Arc::new(Payables {
        db: s.get::<Db>("base/database.pool")?.as_ref().clone(),
        ledger: s.get(bilibi_finance_ledger::SERVICE)?,
    }); agent_operations::bind(payables.clone(), s.get("base/tenancy.parties")?)?; Ok(payables) } }],
} }

pub type Result<T> = std::result::Result<T, PayablesError>;
#[derive(Debug, thiserror::Error)]
pub enum PayablesError {
    #[error("Access denied")]
    Unauthorized,
    #[error("Not found")]
    CompanyNotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    #[error("Payables are not configured")]
    PayablesNotConfigured,
    #[error("Not found")]
    DocumentNotFound,
    #[error("Not found")]
    TaxCodeNotFound,
    #[error("The document is not a draft")]
    NotDraft,
    #[error("The document is not approved")]
    NotApproved,
    #[error("The document cannot be voided")]
    NotVoidable,
    #[error("The corrected document is not a posted bill")]
    NotPostedBill,
    #[error("The amount exceeds what is outstanding")]
    ExceedsOutstanding,
    #[error("Not found")]
    PaymentRequestNotFound,
    #[error("The payment request is already decided")]
    PaymentRequestDecided,
    #[error("Not found")]
    PaymentNotFound,
    #[error("The payment is already reversed")]
    AlreadyReversed,
    #[error("A reversal cannot be reversed")]
    ReversalNotReversible,
    #[error("Invalid option")]
    InvalidOption,
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
    #[error(transparent)]
    Failed(#[from] Error),
}
impl From<tokio_postgres::Error> for PayablesError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Failed(e.into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToleranceKind {
    Percent,
    Amount,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tolerance {
    pub kind: ToleranceKind,
    pub value: Decimal,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    pub payable_account_id: i64,
    pub quantity_tolerance: Option<Tolerance>,
    pub price_tolerance: Option<Tolerance>,
}
#[derive(Clone, Debug)]
pub struct SettingsInput {
    pub settings: Settings,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaxCode {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub rate: Decimal,
    pub account_id: i64,
    pub active: bool,
}
#[derive(Clone, Debug)]
pub struct NewTaxCode {
    pub code: String,
    pub details: TaxCodeDetails,
}
#[derive(Clone, Debug)]
pub struct TaxCodeDetails {
    pub name: String,
    pub rate: String,
    pub account_id: i64,
    pub active: bool,
    pub reason: Option<String>,
}
/// Histories retain the reference's explicit `old_*` and `new_*` values,
/// reason, actor, impersonator and database timestamp.
#[derive(Clone, Debug)]
pub struct HistoryPage {
    pub rows: Vec<serde_json::Value>,
    pub next_after: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Party {
    pub key: String,
    pub name: String,
    pub tax_id: Option<String>,
    pub registration_id: Option<String>,
    pub address: Option<String>,
}
#[derive(Clone, Debug)]
pub struct NewDocument {
    pub party: Party,
    pub party_document_number: Option<String>,
    pub document_date: Date,
    pub due_date: Option<Date>,
    pub description: Option<String>,
    pub lines: Vec<DocumentLineInput>,
    /// None computes tax. Some overrides amounts for the used codes the party states.
    pub tax_lines: Option<Vec<TaxAmountInput>>,
}
#[derive(Clone, Debug)]
pub struct DocumentLineInput {
    pub description: String,
    pub account_id: i64,
    pub amount: Money,
    pub tax_code_id: Option<i64>,
    pub external_order_line_id: Option<String>,
    pub order_line_id: Option<i64>,
    pub quantity: Option<Decimal>,
}
#[derive(Clone, Debug)]
pub struct TaxAmountInput {
    pub tax_code_id: i64,
    pub tax_amount: Money,
}
#[derive(Clone, Debug, Serialize)]
pub struct DocumentLine {
    pub id: i64,
    pub line_number: i32,
    pub description: String,
    pub account_id: i64,
    pub amount: Money,
    pub tax_code_id: Option<i64>,
    pub external_order_line_id: Option<String>,
    pub order_line_id: Option<i64>,
    pub quantity: Option<Decimal>,
}
#[derive(Clone, Debug, Serialize)]
pub struct TaxLine {
    pub id: i64,
    pub line_number: i32,
    pub tax_code_id: i64,
    pub tax_code: String,
    pub rate: Decimal,
    pub account_id: i64,
    pub taxable_amount: Money,
    pub tax_amount: Money,
}
#[derive(Clone, Debug, Serialize)]
pub struct Document {
    pub id: i64,
    pub document_type: String,
    pub status: String,
    pub document_number: Option<i64>,
    pub corrects_document_id: Option<i64>,
    pub party: Party,
    pub party_document_number: Option<String>,
    #[serde(serialize_with = "as_text")]
    pub document_date: Date,
    #[serde(serialize_with = "as_text")]
    pub due_date: Date,
    pub currency_code: String,
    pub description: Option<String>,
    pub total_net: Money,
    pub total_tax: Money,
    pub total: Money,
    pub created_by_user_id: i64,
    pub created_by_impersonator_id: Option<i64>,
    pub created_at: String,
    pub approved_by_user_id: Option<i64>,
    pub approved_by_impersonator_id: Option<i64>,
    pub approved_at: Option<String>,
    pub posted_by_user_id: Option<i64>,
    pub posted_by_impersonator_id: Option<i64>,
    pub posted_at: Option<String>,
    pub voided_by_user_id: Option<i64>,
    pub voided_by_impersonator_id: Option<i64>,
    pub voided_at: Option<String>,
    pub void_reason: Option<String>,
    pub payable_account_id: Option<i64>,
    pub entry_id: Option<i64>,
    pub entry_number: Option<i64>,
    pub lines: Vec<DocumentLine>,
    pub tax_lines: Vec<TaxLine>,
    pub debit_note_ids: Vec<i64>,
    pub outstanding: Option<Money>,
}
impl Document {
    pub fn label(&self) -> String {
        let kind = if self.document_type == "bill" {
            "Bill"
        } else {
            "Debit note"
        };
        match self.document_number {
            Some(n) => format!("{kind} {n}"),
            None => format!("{kind} (record {})", self.id),
        }
    }
}
#[derive(Clone, Debug)]
pub struct DocumentQuery {
    pub document_type: Option<String>,
    pub status: Option<String>,
    pub party_key: Option<String>,
    pub from: Option<Date>,
    pub to: Option<Date>,
    pub after: Option<(Date, i64)>,
    pub limit: i64,
}
impl Default for DocumentQuery {
    fn default() -> Self {
        Self {
            document_type: None,
            status: None,
            party_key: None,
            from: None,
            to: None,
            after: None,
            limit: 100,
        }
    }
}
#[derive(Clone, Debug)]
pub struct DocumentPageQuery {
    pub document_type: Option<String>,
    pub status: Option<String>,
    pub search: String,
    pub page: i64,
    pub page_size: i64,
}
impl Default for DocumentPageQuery {
    fn default() -> Self {
        Self {
            document_type: None,
            status: None,
            search: String::new(),
            page: 1,
            page_size: 25,
        }
    }
}
#[derive(Clone, Debug)]
pub struct DocumentPage {
    pub documents: Vec<Document>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}

pub struct Payables {
    db: Db,
    ledger: Arc<Ledger>,
}
impl Payables {
    async fn write<'c>(
        &self,
        client: &'c mut tokio_postgres::Client,
        scope: &Scope,
        company: i64,
        capability: &str,
        action: &str,
    ) -> Result<MutationTransaction<'c>> {
        self.ledger
            .write(client, scope, company, &[capability], action)
            .await
            .map_err(|e| match e {
                LedgerError::Unauthorized => PayablesError::Unauthorized,
                LedgerError::CompanyNotFound => PayablesError::CompanyNotFound,
                LedgerError::CompanyArchived => PayablesError::CompanyArchived,
                e => PayablesError::Ledger(e),
            })
    }
}
async fn authorize(c: &impl GenericClient, s: &Scope, company: i64, cap: &str) -> Result<()> {
    organisation::authorize_target(c, s, company, cap)
        .await
        .map(|_| ())
        .map_err(refusal)
}
fn refusal(e: Error) -> PayablesError {
    match e {
        Error::Forbidden | Error::Unauthorized => PayablesError::Unauthorized,
        Error::NotFound => PayablesError::CompanyNotFound,
        e => PayablesError::Failed(e),
    }
}
fn invalid(field: &'static str, message: impl Into<String>) -> PayablesError {
    PayablesError::Invalid(vec![FieldError::new(field, message)])
}

fn as_text<S: serde::Serializer>(
    date: &Date,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&date.to_string())
}

async fn join(tx: &MutationTransaction<'_>, s: &Scope, company: i64, cap: &str) -> Result<()> {
    authorize(tx.sql(), s, company, cap).await?;
    organisation::require_writable(tx.sql(), s, company, true)
        .await
        .map(|_| ())
        .map_err(|e| match e {
            bilibi_company::WriteError::Archived => PayablesError::CompanyArchived,
            bilibi_company::WriteError::NotFound => PayablesError::CompanyNotFound,
            bilibi_company::WriteError::Access(e) => refusal(e),
            _ => PayablesError::Failed(Error::Internal),
        })
}
