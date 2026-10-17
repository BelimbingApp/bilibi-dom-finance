//! The clearance authority a company's documents go to (Bilimbi
//! `Bilimbi.Finance.Einvoice.Authority`).
//!
//! The facade, tables and pages speak only of fiscal documents, parties,
//! submissions and status events. Everything one authority defines (its rules,
//! code lists, document format, signature, API and the shape of its answers)
//! sits behind this trait in that authority's own module. This release clears
//! documents with one authority, MyInvois (`myinvois.rs`).
//!
//! The callbacks that take a draft or a recorded document (draft validation
//! and the payload) are here with their types; the API callbacks (login,
//! submit, progress, find, status, cancel) join the trait with the MyInvois
//! client, and the consolidated document's shape and refusal with
//! consolidation, each with the types it needs; none of them is invented
//! before then.
use crate::code_lists::Codes;
use crate::document::Document;
use crate::draft::Draft;
use crate::myinvois::signer::SigningKey;
use crate::FieldError;
use jiff::Timestamp;
use std::collections::BTreeMap;

/// A document format version a company may choose, and whether it carries a
/// signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentVersion {
    pub version: &'static str,
    pub signed: bool,
}

/// The ledger accounts a validated document posts to, by the company's own
/// account codes (the `finance.einvoice.posting.*` settings).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PostingAccount {
    Receivable,
    Revenue,
    Tax,
    Settlement,
    Payable,
    Expense,
    PurchaseTax,
}
impl PostingAccount {
    pub const ALL: [PostingAccount; 7] = [
        Self::Receivable,
        Self::Revenue,
        Self::Tax,
        Self::Settlement,
        Self::Payable,
        Self::Expense,
        Self::PurchaseTax,
    ];
    /// The setting field name: `finance.einvoice.posting.<field>`.
    pub fn field(self) -> &'static str {
        match self {
            Self::Receivable => "receivable_account",
            Self::Revenue => "revenue_account",
            Self::Tax => "tax_account",
            Self::Settlement => "settlement_account",
            Self::Payable => "payable_account",
            Self::Expense => "expense_account",
            Self::PurchaseTax => "purchase_tax_account",
        }
    }
}

/// The side of an entry line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Debit,
    Credit,
}

/// How a validated document of one type posts to the ledger, naming the
/// company's posting settings: `counter` (the receivable, payable or
/// settlement account) takes `side`, and `net` (revenue or expense) and `tax`
/// take the opposite side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostingRule {
    pub side: Side,
    pub counter: PostingAccount,
    pub net: PostingAccount,
    pub tax: PostingAccount,
}

/// A field whose shape the authority fixes, for settings validators and
/// document validation alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormatKind {
    TaxId,
    Phone,
    IndustryCode,
    RegistrationType,
    StateCode,
    CountryCode,
}

/// The supplier fields every document carries that a company's profile can
/// leave blank.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SupplierField {
    TaxId,
    RegistrationNumber,
    IndustryCode,
    BusinessActivity,
    Phone,
    AddressLine1,
    City,
    StateCode,
    CountryCode,
}
impl SupplierField {
    /// The name of the `finance.einvoice.supplier.*` setting that fills it.
    pub fn field(self) -> &'static str {
        match self {
            Self::TaxId => "tax_id",
            Self::RegistrationNumber => "registration_number",
            Self::IndustryCode => "industry_code",
            Self::BusinessActivity => "business_activity",
            Self::Phone => "phone",
            Self::AddressLine1 => "address_line1",
            Self::City => "city",
            Self::StateCode => "state_code",
            Self::CountryCode => "country_code",
        }
    }
}

/// The attributes of a signing certificate's subject that carry the
/// company's tax and registration numbers, as the authority writes them
/// there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CertificateSubject {
    pub organization_identifier: Option<String>,
    pub serial_number: Option<String>,
}

/// The tax ID and registration number a certificate states.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateIdentity {
    pub tax_id: Option<String>,
    pub registration_number: Option<String>,
}

/// The company's supplier profile as the authority needs it: every
/// `finance.einvoice.supplier.*` value, blank ones as `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Supplier {
    pub name: Option<String>,
    pub tax_id: Option<String>,
    pub registration_type: Option<String>,
    pub registration_number: Option<String>,
    pub sales_tax_registration: Option<String>,
    pub tourism_tax_registration: Option<String>,
    pub industry_code: Option<String>,
    pub business_activity: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address_line1: Option<String>,
    pub address_line2: Option<String>,
    pub address_line3: Option<String>,
    pub city: Option<String>,
    pub postcode: Option<String>,
    pub state_code: Option<String>,
    pub country_code: Option<String>,
}

/// The period a consolidated document covers and when it may be issued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsolidationWindow {
    /// The first instant of the month, in the authority's calendar.
    pub from: Timestamp,
    /// The first instant after the month, and the moment from which the
    /// consolidated document may be issued.
    pub until: Timestamp,
    /// When the time the authority allows for issuing it ends.
    pub closes_at: Timestamp,
}

/// What the authority's draft validation reads beside the draft.
#[derive(Clone, Copy, Debug)]
pub struct ValidationContext<'a> {
    /// The company's own profile, which is the supplier, or the buyer of a
    /// self-billed document.
    pub supplier: &'a Supplier,
    /// The company's active codes, one entry per list the authority
    /// publishes (`Einvoice::list_codes`); a list missing here knows no code.
    pub codes: &'a BTreeMap<String, Codes>,
    /// The document a note's `original_document_id` names, as the company
    /// recorded it, or `None` when it names none of the company's.
    pub original: Option<&'a Original>,
    /// Whether the draft is a consolidated document built by consolidation.
    pub consolidated: bool,
}

/// What draft validation needs of the original document a note corrects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Original {
    pub document_type: String,
    pub currency_code: String,
    /// The document's current status (`valid`, `rejected`, ...).
    pub status: String,
    /// The authority's UUID for it, once validated.
    pub document_uuid: Option<String>,
}

/// How a document's payload is built.
#[derive(Clone, Copy, Debug)]
pub struct PayloadOptions<'a> {
    /// The format version, one of [`Authority::document_versions`].
    pub version: &'a str,
    /// The company's certificate and key, for a version that is signed.
    pub signing_key: Option<&'a SigningKey>,
}

/// A document's payload: the exact bytes to submit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Payload {
    /// `JSON` or `XML`.
    pub format: &'static str,
    pub version: String,
    pub bytes: Vec<u8>,
}

/// Why a payload was not built.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PayloadError {
    /// The version is signed and no signing key was given.
    #[error("signing key required")]
    SigningKeyRequired,
    /// Not a version the authority accepts.
    #[error("unsupported version")]
    UnsupportedVersion,
    /// The private key did not sign.
    #[error("the private key did not sign")]
    SigningFailed,
}

/// What one clearance authority defines for the module.
pub trait Authority {
    /// The authority's stable ID, stored on each document and code override.
    fn id(&self) -> &'static str;

    /// The document types a company may issue.
    fn issuable_document_types(&self) -> Vec<&'static str>;

    /// The document types that reference an original document (credit, debit
    /// and refund notes).
    fn note_document_types(&self) -> Vec<&'static str>;

    /// The document types the company issues as the buyer (self-billed): the
    /// counterparty is the supplier and the company's own profile the buyer.
    fn self_billed_document_types(&self) -> Vec<&'static str>;

    /// How a validated document of this type posts to the ledger, or `None`
    /// for a type with no rule.
    fn posting_rule(&self, document_type: &str) -> Option<PostingRule>;

    /// The human name of a document type, as the pages and the PDF show it.
    fn document_type_name(&self, document_type: &str) -> String;

    /// Hours after validation in which the supplier may still cancel a document.
    fn cancellation_window_hours(&self) -> u32;

    /// Hours of unbroken outage after which an operator is told a submission
    /// is overdue.
    fn outage_notice_hours(&self) -> u32;

    /// The document format versions a company may choose, the default first.
    fn document_versions(&self) -> &'static [DocumentVersion];

    /// Whether `value` has the shape the authority requires of a field of
    /// this kind. Settings validators and document validation both ask here.
    fn valid_format(&self, kind: FormatKind, value: &str) -> bool;

    /// What `valid_format` requires of a field of this kind, for an error
    /// message.
    fn format_message(&self, kind: FormatKind) -> &'static str;

    /// The supplier profile fields every document carries that `supplier`
    /// leaves blank.
    fn supplier_missing(&self, supplier: &Supplier) -> Vec<SupplierField>;

    /// The tax ID and registration number a signing certificate's subject
    /// carries.
    fn certificate_identity(&self, subject: &CertificateSubject) -> CertificateIdentity;

    /// The period a consolidated document for the calendar month containing
    /// `month` covers, in the authority's own calendar, and when it may be
    /// issued.
    fn consolidation_window(&self, month: jiff::civil::Date) -> ConsolidationWindow;

    /// The public link that shows a validated document on the authority's
    /// portal, whose base URL is `portal_url`.
    fn validation_url(&self, portal_url: &str, document_uuid: &str, long_id: &str) -> String;

    /// The authority's own refusals of a draft, beside the shape check every
    /// authority shares (`Draft::validate` runs both): codes outside the
    /// company's code lists, fields its rules require, a company profile
    /// missing what every document carries, and a note whose original is not
    /// one the authority lets it reference.
    fn validate(&self, draft: &Draft, context: &ValidationContext<'_>) -> Vec<FieldError>;

    /// The payload for a recorded document in `options.version` of the
    /// authority's format: its format, version and the exact bytes to submit,
    /// signed at `signing_time` when the version is signed.
    fn payload(
        &self,
        document: &Document,
        options: &PayloadOptions<'_>,
        signing_time: Timestamp,
    ) -> Result<Payload, PayloadError>;
}
