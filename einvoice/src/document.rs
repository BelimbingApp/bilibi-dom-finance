//! A recorded fiscal document, as the authority's payload reads it (Bilimbi
//! `Einvoice.get_document/3`'s map): the `finance_einvoice_documents` row
//! with its lines, the party snapshots the issue took, and the details a
//! note or a billing period adds. The facade's reads return this record from
//! the next version of this module on; the walkthrough builds one by hand
//! from a valid draft until then.
use crate::authority::Supplier;
use crate::draft::ValidParty;
use jiff::Timestamp;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// A recorded document with its lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Document {
    pub id: i64,
    /// `sandbox` or `production`.
    pub environment: String,
    pub document_type: String,
    pub code_number: String,
    pub issued_at: Timestamp,
    pub currency_code: String,
    pub exchange_rate: Option<Decimal>,
    /// The supplier as the issue snapshotted it: the company's profile, or
    /// the counterparty of a self-billed document.
    pub supplier: PartySnapshot,
    /// The buyer as the issue snapshotted it.
    pub buyer: PartySnapshot,
    pub details: Details,
    pub original_document_id: Option<i64>,
    pub line_extension_amount: Decimal,
    pub tax_exclusive_amount: Decimal,
    pub tax_amount: Decimal,
    pub tax_inclusive_amount: Decimal,
    pub payable_amount: Decimal,
    pub lines: Vec<DocumentLine>,
}

/// One recorded line (`finance_einvoice_document_lines`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DocumentLine {
    pub line_number: i32,
    pub description: String,
    pub classification_code: String,
    pub tariff_code: Option<String>,
    pub origin_country_code: Option<String>,
    pub quantity: Decimal,
    pub unit_code: Option<String>,
    pub unit_price: Decimal,
    pub subtotal: Decimal,
    pub discount_amount: Decimal,
    pub charge_amount: Decimal,
    pub line_extension_amount: Decimal,
    pub tax_type: String,
    pub tax_rate: Option<Decimal>,
    pub tax_amount: Decimal,
    pub tax_exemption_reason: Option<String>,
}

/// A party as the issue snapshotted it into the document's `supplier` or
/// `buyer` column, with Bilimbi's keys.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartySnapshot {
    pub name: Option<String>,
    pub tax_id: Option<String>,
    pub registration_type: Option<String>,
    pub registration_number: Option<String>,
    pub sales_tax_registration: Option<String>,
    #[serde(default)]
    pub tourism_tax_registration: Option<String>,
    #[serde(default)]
    pub industry_code: Option<String>,
    #[serde(default)]
    pub business_activity: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub address: AddressSnapshot,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddressSnapshot {
    #[serde(default)]
    pub lines: Vec<String>,
    pub city: Option<String>,
    pub postcode: Option<String>,
    pub state_code: Option<String>,
    pub country_code: Option<String>,
}

/// What the issue keeps beside the document in its `details` column: the
/// payment and billing-period fields of the draft, and for a note the
/// original's code number and UUID as they stood when the note was issued.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Details {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_means_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payment_terms: Option<String>,
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_period_start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_period_end: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_period_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_code_number: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_document_uuid: Option<String>,
    /// The month (`YYYY-MM`) a consolidated document reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consolidation_month: Option<String>,
}

impl PartySnapshot {
    /// The company's own profile as the issue snapshots it (the supplier of
    /// a sale, the buyer of a self-billed document).
    pub fn of_profile(profile: &Supplier) -> PartySnapshot {
        PartySnapshot {
            name: profile.name.clone(),
            tax_id: profile.tax_id.clone(),
            registration_type: profile.registration_type.clone(),
            registration_number: profile.registration_number.clone(),
            sales_tax_registration: profile.sales_tax_registration.clone(),
            tourism_tax_registration: profile.tourism_tax_registration.clone(),
            industry_code: profile.industry_code.clone(),
            business_activity: profile.business_activity.clone(),
            phone: profile.phone.clone(),
            email: profile.email.clone(),
            address: AddressSnapshot {
                lines: [
                    &profile.address_line1,
                    &profile.address_line2,
                    &profile.address_line3,
                ]
                .into_iter()
                .flatten()
                .cloned()
                .collect(),
                city: profile.city.clone(),
                postcode: profile.postcode.clone(),
                state_code: profile.state_code.clone(),
                country_code: profile.country_code.clone(),
            },
        }
    }

    /// The counterparty of a valid draft as the issue snapshots it: the buyer
    /// of a sale, or (`supplier_fields`) the supplier of a self-billed
    /// document, which carries a supplier's industry and activity too.
    pub fn of_party(party: &ValidParty, supplier_fields: bool) -> PartySnapshot {
        PartySnapshot {
            name: Some(party.name.clone()),
            tax_id: Some(party.tax_id.clone()),
            registration_type: Some(party.registration_type.clone()),
            registration_number: Some(party.registration_number.clone()),
            sales_tax_registration: party.sales_tax_registration.clone(),
            tourism_tax_registration: None,
            industry_code: supplier_fields
                .then(|| party.industry_code.clone())
                .flatten(),
            business_activity: supplier_fields
                .then(|| party.business_activity.clone())
                .flatten(),
            phone: party.phone.clone(),
            email: party.email.clone(),
            address: AddressSnapshot {
                lines: party.address.lines.clone(),
                city: Some(party.address.city.clone()),
                postcode: party.address.postcode.clone(),
                state_code: Some(party.address.state_code.clone()),
                country_code: Some(party.address.country_code.clone()),
            },
        }
    }
}
