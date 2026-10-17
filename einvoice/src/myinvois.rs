//! The MyInvois clearance authority (Bilimbi `Bilimbi.Finance.Einvoice.MyInvois`):
//! what the module knows of it without talking to it. Rules follow the
//! authority's e-Invoice Guideline v4.8 (30 August 2026) and its SDK as read on
//! 12 October 2026. Format version 1.1 turns on signature validation; version
//! 1.0 is submitted unsigned, which is how a company tests the sandbox before
//! its certificate arrives. Invoices (`01`) and the credit, debit and refund
//! notes that reference a validated original (`02` to `04`), their
//! self-billed counterparts (`11` to `14`), which the company issues as the
//! buyer, and the monthly consolidated invoice, in ringgit, are issued in this
//! release; foreign currencies come later.
//!
//! The consolidated invoice follows the Specific Guideline v4.9 (§3.6, §3.7):
//! it reports a calendar month (Malaysian time) of sales no buyer asked an
//! individual e-invoice for, issued within seven days after the month ends. A
//! self-billed document (§8.3) names the supplier as the counterparty.
use crate::authority::{
    Authority, CertificateIdentity, CertificateSubject, ConsolidationWindow, DocumentVersion,
    FormatKind, PostingAccount, PostingRule, Side, Supplier, SupplierField,
};
use jiff::{civil::Date, tz::TimeZone, SignedDuration, Timestamp};

/// The authority's stable ID.
pub(crate) const ID: &str = "myinvois";
/// The format version documents are built in when a company chose none.
pub(crate) const VERSION: &str = "1.1";
const VERSIONS: [DocumentVersion; 2] = [
    DocumentVersion {
        version: "1.1",
        signed: true,
    },
    DocumentVersion {
        version: "1.0",
        signed: false,
    },
];

const INVOICE: &str = "01";
const NOTES: [&str; 3] = ["02", "03", "04"];
const SELF_BILLED_INVOICE: &str = "11";
const SELF_BILLED_NOTES: [&str; 3] = ["12", "13", "14"];

// A consolidated invoice is issued within seven days after its month ends, in
// Malaysian time (UTC+8, no daylight saving).
const LOCAL_OFFSET_HOURS: i8 = 8;
const CONSOLIDATION_DAYS: i64 = 7;

pub(crate) const TAX_ID_MESSAGE: &str = "must be one or two capital letters and 8 to 12 digits";
pub(crate) const PHONE_MESSAGE: &str =
    "must be a plus sign and 8 to 20 digits, 20 characters at most";
pub(crate) const INDUSTRY_CODE_MESSAGE: &str = "must be five digits";
pub(crate) const REGISTRATION_TYPE_MESSAGE: &str = "must be BRN, NRIC, PASSPORT, or ARMY";
pub(crate) const STATE_CODE_MESSAGE: &str = "must be two digits";
pub(crate) const COUNTRY_CODE_MESSAGE: &str = "must be three capital letters";

/// MyInvois, the one authority this release clears documents with.
#[derive(Clone, Copy, Debug)]
pub(crate) struct MyInvois;

fn digits(value: &[u8], count: std::ops::RangeInclusive<usize>) -> bool {
    count.contains(&value.len()) && value.iter().all(u8::is_ascii_digit)
}

pub(crate) fn valid_format(kind: FormatKind, value: &str) -> bool {
    let bytes = value.as_bytes();
    match kind {
        // One or two capital letters, then 8 to 12 digits.
        FormatKind::TaxId => {
            let letters = bytes.iter().take_while(|b| b.is_ascii_uppercase()).count();
            (1..=2).contains(&letters) && digits(&bytes[letters..], 8..=12)
        }
        FormatKind::Phone => bytes.first() == Some(&b'+') && digits(&bytes[1..], 8..=19),
        FormatKind::IndustryCode => digits(bytes, 5..=5),
        FormatKind::RegistrationType => ["BRN", "NRIC", "PASSPORT", "ARMY"].contains(&value),
        FormatKind::StateCode => digits(bytes, 2..=2),
        FormatKind::CountryCode => bytes.len() == 3 && bytes.iter().all(u8::is_ascii_uppercase),
    }
}

/// Whether the format version is one the authority accepts.
pub(crate) fn valid_document_version(value: &str) -> bool {
    VERSIONS.iter().any(|v| v.version == value)
}

/// Settings validator for the authority's base URLs: an https URL without a
/// path (a query or fragment is not read).
pub(crate) fn valid_url(value: &str) -> bool {
    let rest = value.strip_prefix("https://").unwrap_or("");
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let host = rest.strip_suffix('/').unwrap_or(rest);
    !host.is_empty()
        && !host.contains('/')
        && !host.chars().any(|c| c.is_whitespace() || c.is_control())
}

impl Authority for MyInvois {
    fn id(&self) -> &'static str {
        ID
    }

    fn issuable_document_types(&self) -> Vec<&'static str> {
        let mut types = vec![INVOICE];
        types.extend(NOTES);
        types.push(SELF_BILLED_INVOICE);
        types.extend(SELF_BILLED_NOTES);
        types
    }

    fn note_document_types(&self) -> Vec<&'static str> {
        NOTES.into_iter().chain(SELF_BILLED_NOTES).collect()
    }

    fn self_billed_document_types(&self) -> Vec<&'static str> {
        let mut types = vec![SELF_BILLED_INVOICE];
        types.extend(SELF_BILLED_NOTES);
        types
    }

    // A sale debits the receivable and credits revenue and tax; a self-billed
    // purchase credits the payable and debits expense and purchase tax. A
    // credit note posts the mirror image, and a refund note settles in cash.
    fn posting_rule(&self, document_type: &str) -> Option<PostingRule> {
        use PostingAccount::{Expense, Payable, PurchaseTax, Receivable, Revenue, Settlement, Tax};
        let sale = |side, counter| PostingRule {
            side,
            counter,
            net: Revenue,
            tax: Tax,
        };
        let purchase = |side, counter| PostingRule {
            side,
            counter,
            net: Expense,
            tax: PurchaseTax,
        };
        match document_type {
            "01" => Some(sale(Side::Debit, Receivable)),
            "02" => Some(sale(Side::Credit, Receivable)),
            "03" => Some(sale(Side::Debit, Receivable)),
            "04" => Some(sale(Side::Credit, Settlement)),
            "11" => Some(purchase(Side::Credit, Payable)),
            "12" => Some(purchase(Side::Debit, Payable)),
            "13" => Some(purchase(Side::Credit, Payable)),
            "14" => Some(purchase(Side::Debit, Settlement)),
            _ => None,
        }
    }

    fn document_type_name(&self, document_type: &str) -> String {
        match document_type {
            "01" => "Invoice".into(),
            "02" => "Credit note".into(),
            "03" => "Debit note".into(),
            "04" => "Refund note".into(),
            "11" => "Self-billed invoice".into(),
            "12" => "Self-billed credit note".into(),
            "13" => "Self-billed debit note".into(),
            "14" => "Self-billed refund note".into(),
            other => format!("Document type {other}"),
        }
    }

    fn cancellation_window_hours(&self) -> u32 {
        72
    }

    fn outage_notice_hours(&self) -> u32 {
        72
    }

    fn document_versions(&self) -> &'static [DocumentVersion] {
        &VERSIONS
    }

    fn valid_format(&self, kind: FormatKind, value: &str) -> bool {
        valid_format(kind, value)
    }

    fn format_message(&self, kind: FormatKind) -> &'static str {
        match kind {
            FormatKind::TaxId => TAX_ID_MESSAGE,
            FormatKind::Phone => PHONE_MESSAGE,
            FormatKind::IndustryCode => INDUSTRY_CODE_MESSAGE,
            FormatKind::RegistrationType => REGISTRATION_TYPE_MESSAGE,
            FormatKind::StateCode => STATE_CODE_MESSAGE,
            FormatKind::CountryCode => COUNTRY_CODE_MESSAGE,
        }
    }

    fn supplier_missing(&self, supplier: &Supplier) -> Vec<SupplierField> {
        [
            (SupplierField::TaxId, &supplier.tax_id),
            (
                SupplierField::RegistrationNumber,
                &supplier.registration_number,
            ),
            (SupplierField::IndustryCode, &supplier.industry_code),
            (SupplierField::BusinessActivity, &supplier.business_activity),
            (SupplierField::Phone, &supplier.phone),
            (SupplierField::AddressLine1, &supplier.address_line1),
            (SupplierField::City, &supplier.city),
            (SupplierField::StateCode, &supplier.state_code),
            (SupplierField::CountryCode, &supplier.country_code),
        ]
        .into_iter()
        .filter(|(_, value)| value.is_none())
        .map(|(field, _)| field)
        .collect()
    }

    fn certificate_identity(&self, subject: &CertificateSubject) -> CertificateIdentity {
        CertificateIdentity {
            tax_id: subject.organization_identifier.clone(),
            registration_number: subject.serial_number.clone(),
        }
    }

    fn consolidation_window(&self, month: Date) -> ConsolidationWindow {
        let first = month.first_of_month();
        let until = local_midnight(last_of_month_plus_one(month));
        ConsolidationWindow {
            from: local_midnight(first),
            until,
            closes_at: until
                .checked_add(SignedDuration::from_hours(CONSOLIDATION_DAYS * 24))
                .unwrap_or(until),
        }
    }

    fn validation_url(&self, portal_url: &str, document_uuid: &str, long_id: &str) -> String {
        format!(
            "{}/{document_uuid}/share/{long_id}",
            portal_url.trim_end_matches('/')
        )
    }
}

fn last_of_month_plus_one(month: Date) -> Date {
    let last = month.last_of_month();
    last.tomorrow().unwrap_or(last)
}

// Midnight of `date` in Malaysian time, as the UTC instant it is.
fn local_midnight(date: Date) -> Timestamp {
    date.at(0, 0, 0, 0)
        .to_zoned(TimeZone::fixed(jiff::tz::offset(LOCAL_OFFSET_HOURS)))
        .map(|zoned| zoned.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}
