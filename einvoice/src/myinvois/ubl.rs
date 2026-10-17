//! UBL 2.1 JSON for the MyInvois authority, from a plain document (Bilimbi
//! `Bilimbi.Finance.Einvoice.MyInvois.Builder`).
//!
//! The builder maps; it does not compute or judge. Every amount arrives
//! already calculated and every code already checked (the draft validation
//! does that), so the same input always yields the same bytes, in the element
//! order of the authority's SDK samples. The `sdk_samples` example rebuilds
//! those samples byte for byte; it can, because the builder reproduces even
//! figures that do not add up.
//!
//! Tax amounts are written in the tax currency, every other amount in the
//! document currency. An optional element is written only when its input is
//! present.
use super::json::Json;
use rust_decimal::Decimal;

/// The builder's input: one document in the authority's format.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Document {
    /// The code number.
    pub id: String,
    /// `YYYY-MM-DD`.
    pub issue_date: String,
    /// `HH:MM:SSZ`, UTC.
    pub issue_time: String,
    pub type_code: String,
    /// The format version, written as the type code's `listVersionID`.
    pub version: String,
    pub currency_code: String,
    pub tax_currency_code: Option<String>,
    pub tax_exchange_rate: Option<ExchangeRate>,
    pub invoice_period: Option<Period>,
    pub billing_references: Vec<BillingReference>,
    pub additional_document_references: Vec<DocumentReference>,
    pub supplier: Party,
    pub buyer: Party,
    pub delivery: Option<Delivery>,
    pub payment_means: Option<PaymentMeans>,
    pub payment_terms: Option<String>,
    pub prepaid_payment: Option<PrepaidPayment>,
    /// Document-level allowances and charges.
    pub allowance_charges: Vec<AllowanceCharge>,
    pub tax_total: TaxTotal,
    pub totals: Totals,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExchangeRate {
    pub source: String,
    pub target: String,
    pub rate: Option<Decimal>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Period {
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub description: Option<String>,
}

/// An additional document reference (`uuid: None`), or the original a note
/// corrects, referenced by its ID and the authority's UUID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BillingReference {
    pub id: String,
    pub uuid: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentReference {
    pub id: String,
    pub document_type: Option<String>,
    pub description: Option<String>,
}

/// A party: the supplier (with its industry code and an optional additional
/// account), the buyer, or a delivery party.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Party {
    pub name: String,
    /// `(scheme, id)` pairs such as `("TIN", ...)`, in order.
    pub identifications: Vec<(String, String)>,
    pub address: Address,
    pub phone: Option<String>,
    pub email: Option<String>,
    /// The supplier's industry classification: `(code, name)`.
    pub industry_code: Option<(String, String)>,
    pub additional_account: Option<AdditionalAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdditionalAccount {
    pub id: String,
    pub scheme_agency_name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Address {
    pub city: String,
    pub postcode: Option<String>,
    pub state_code: String,
    pub lines: Vec<String>,
    pub country_code: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Delivery {
    pub party: Option<Party>,
    pub shipment: Option<Shipment>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shipment {
    pub id: String,
    pub freight_allowance_charge: Option<AllowanceCharge>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaymentMeans {
    pub code: String,
    pub payee_account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrepaidPayment {
    pub id: Option<String>,
    pub amount: Decimal,
    pub date: Option<String>,
    pub time: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowanceCharge {
    /// `true` for a charge, `false` for an allowance.
    pub charge: bool,
    pub reason: Option<String>,
    pub multiplier: Option<Decimal>,
    pub amount: Decimal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaxTotal {
    pub tax_amount: Decimal,
    pub subtotals: Vec<TaxSubtotal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaxSubtotal {
    pub taxable_amount: Decimal,
    pub tax_amount: Decimal,
    pub percent: Option<Decimal>,
    /// A per-unit tax: quantity, unit code and amount.
    pub per_unit: Option<PerUnit>,
    pub tax_type: String,
    pub exemption_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PerUnit {
    pub quantity: Decimal,
    pub unit_code: String,
    pub amount: Decimal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub line_extension_amount: Decimal,
    pub tax_exclusive_amount: Decimal,
    pub tax_inclusive_amount: Decimal,
    pub allowance_total_amount: Option<Decimal>,
    pub charge_total_amount: Option<Decimal>,
    pub payable_rounding_amount: Option<Decimal>,
    pub payable_amount: Decimal,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub id: String,
    pub quantity: Decimal,
    pub unit_code: Option<String>,
    pub line_extension_amount: Decimal,
    pub allowance_charges: Vec<AllowanceCharge>,
    pub tax_total: TaxTotal,
    /// `(list id, code)` pairs such as `("CLASS", "003")`, in order.
    pub classifications: Vec<(String, String)>,
    pub description: Option<String>,
    pub origin_country: Option<String>,
    pub unit_price: Decimal,
    pub subtotal: Decimal,
}

const NAMESPACES: [(&str, &str); 3] = [
    (
        "_D",
        "urn:oasis:names:specification:ubl:schema:xsd:Invoice-2",
    ),
    (
        "_A",
        "urn:oasis:names:specification:ubl:schema:xsd:CommonAggregateComponents-2",
    ),
    (
        "_B",
        "urn:oasis:names:specification:ubl:schema:xsd:CommonBasicComponents-2",
    ),
];

/// Amounts in one currency.
#[derive(Clone, Copy)]
struct Money<'a>(&'a str);

impl Money<'_> {
    fn of(self, amount: Decimal) -> Option<Json> {
        Some(Json::value(
            Json::number(amount),
            &[("currencyID", Some(self.0))],
        ))
    }
    fn maybe(self, amount: Option<Decimal>) -> Option<Json> {
        amount.and_then(|amount| self.of(amount))
    }
}

fn text(value: &str) -> Option<Json> {
    Json::text(Some(value))
}

fn optional(value: Option<&str>) -> Option<Json> {
    Json::text(value)
}

fn number(value: Option<Decimal>) -> Option<Json> {
    value.map(|value| Json::value(Json::number(value), &[]))
}

fn one(item: Json) -> Option<Json> {
    Some(Json::Array(vec![item]))
}

fn many<T>(items: &[T], f: impl Fn(&T) -> Json) -> Option<Json> {
    Some(Json::Array(items.iter().map(f).collect()))
}

/// The UBL document as an ordered JSON term, unsigned.
pub fn build(document: &Document) -> Json {
    let money = Money(&document.currency_code);
    let tax_money = Money(
        document
            .tax_currency_code
            .as_deref()
            .unwrap_or(&document.currency_code),
    );

    let invoice = Json::object([
        ("ID", text(&document.id)),
        ("IssueDate", text(&document.issue_date)),
        ("IssueTime", text(&document.issue_time)),
        (
            "InvoiceTypeCode",
            Some(Json::value(
                Json::str(&document.type_code),
                &[("listVersionID", Some(&document.version))],
            )),
        ),
        ("DocumentCurrencyCode", text(&document.currency_code)),
        (
            "TaxCurrencyCode",
            optional(document.tax_currency_code.as_deref()),
        ),
        (
            "InvoicePeriod",
            document
                .invoice_period
                .as_ref()
                .and_then(|p| one(period(p))),
        ),
        (
            "BillingReference",
            many(&document.billing_references, billing_reference),
        ),
        (
            "AdditionalDocumentReference",
            many(
                &document.additional_document_references,
                additional_reference,
            ),
        ),
        ("AccountingSupplierParty", one(supplier(&document.supplier))),
        (
            "AccountingCustomerParty",
            one(Json::object([("Party", one(buyer(&document.buyer)))])),
        ),
        (
            "Delivery",
            document
                .delivery
                .as_ref()
                .and_then(|d| one(delivery(d, money))),
        ),
        (
            "PaymentMeans",
            document
                .payment_means
                .as_ref()
                .and_then(|m| one(payment_means(m))),
        ),
        (
            "PaymentTerms",
            document
                .payment_terms
                .as_deref()
                .and_then(|note| one(Json::object([("Note", text(note))]))),
        ),
        (
            "PrepaidPayment",
            document
                .prepaid_payment
                .as_ref()
                .and_then(|p| one(prepaid(p, money))),
        ),
        (
            "AllowanceCharge",
            many(&document.allowance_charges, |a| allowance_charge(a, money)),
        ),
        (
            "TaxExchangeRate",
            document
                .tax_exchange_rate
                .as_ref()
                .and_then(|r| one(exchange_rate(r))),
        ),
        (
            "TaxTotal",
            one(tax_total(&document.tax_total, money, tax_money)),
        ),
        ("LegalMonetaryTotal", one(totals(&document.totals, money))),
        (
            "InvoiceLine",
            many(&document.lines, |l| line(l, money, tax_money)),
        ),
    ]);

    let mut pairs: Vec<(&str, Option<Json>)> = NAMESPACES
        .iter()
        .map(|(key, uri)| (*key, Some(Json::str(uri))))
        .collect();
    pairs.push(("Invoice", one(invoice)));
    Json::object(pairs)
}

fn period(period: &Period) -> Json {
    Json::object([
        ("StartDate", optional(period.start_date.as_deref())),
        ("EndDate", optional(period.end_date.as_deref())),
        ("Description", optional(period.description.as_deref())),
    ])
}

fn billing_reference(reference: &BillingReference) -> Json {
    match &reference.uuid {
        Some(uuid) => Json::object([(
            "InvoiceDocumentReference",
            one(Json::object([
                ("ID", text(&reference.id)),
                ("UUID", text(uuid)),
            ])),
        )]),
        None => Json::object([(
            "AdditionalDocumentReference",
            one(Json::object([("ID", text(&reference.id))])),
        )]),
    }
}

fn additional_reference(reference: &DocumentReference) -> Json {
    Json::object([
        ("ID", text(&reference.id)),
        ("DocumentType", optional(reference.document_type.as_deref())),
        (
            "DocumentDescription",
            optional(reference.description.as_deref()),
        ),
    ])
}

// Parties. Each role keeps the element order of the authority's samples.

fn supplier(party: &Party) -> Json {
    let industry = party
        .industry_code
        .as_ref()
        .map(|(code, name)| Json::value(Json::str(code), &[("name", Some(name.as_str()))]));
    Json::object([
        (
            "AdditionalAccountID",
            party.additional_account.as_ref().and_then(|account| {
                one(Json::Object(vec![
                    ("_".to_owned(), Json::str(&account.id)),
                    (
                        "schemeAgencyName".to_owned(),
                        Json::str(&account.scheme_agency_name),
                    ),
                ]))
            }),
        ),
        (
            "Party",
            one(Json::object([
                ("IndustryClassificationCode", industry),
                ("PartyIdentification", identifications(party)),
                ("PostalAddress", one(address(&party.address))),
                ("PartyLegalEntity", legal_entity(party)),
                ("Contact", contact(party)),
            ])),
        ),
    ])
}

fn buyer(party: &Party) -> Json {
    Json::object([
        ("PostalAddress", one(address(&party.address))),
        ("PartyLegalEntity", legal_entity(party)),
        ("PartyIdentification", identifications(party)),
        ("Contact", contact(party)),
    ])
}

fn delivery_party(party: &Party) -> Json {
    Json::object([
        ("PartyLegalEntity", legal_entity(party)),
        ("PostalAddress", one(address(&party.address))),
        ("PartyIdentification", identifications(party)),
    ])
}

fn identifications(party: &Party) -> Option<Json> {
    many(&party.identifications, |(scheme, id)| {
        Json::object([(
            "ID",
            Some(Json::value(Json::str(id), &[("schemeID", Some(scheme))])),
        )])
    })
}

fn legal_entity(party: &Party) -> Option<Json> {
    one(Json::object([("RegistrationName", text(&party.name))]))
}

fn contact(party: &Party) -> Option<Json> {
    let contact = Json::object([
        ("Telephone", optional(party.phone.as_deref())),
        ("ElectronicMail", optional(party.email.as_deref())),
    ]);
    match &contact {
        Json::Object(pairs) if pairs.is_empty() => None,
        _ => one(contact),
    }
}

fn address(address: &Address) -> Json {
    Json::object([
        ("CityName", text(&address.city)),
        ("PostalZone", optional(address.postcode.as_deref())),
        ("CountrySubentityCode", text(&address.state_code)),
        (
            "AddressLine",
            many(&address.lines, |line| Json::object([("Line", text(line))])),
        ),
        (
            "Country",
            one(Json::object([(
                "IdentificationCode",
                Some(Json::value(
                    Json::str(&address.country_code),
                    &[("listID", Some("ISO3166-1")), ("listAgencyID", Some("6"))],
                )),
            )])),
        ),
    ])
}

fn delivery(delivery: &Delivery, money: Money<'_>) -> Json {
    Json::object([
        (
            "DeliveryParty",
            delivery.party.as_ref().and_then(|p| one(delivery_party(p))),
        ),
        (
            "Shipment",
            delivery
                .shipment
                .as_ref()
                .and_then(|s| one(shipment(s, money))),
        ),
    ])
}

fn shipment(shipment: &Shipment, money: Money<'_>) -> Json {
    Json::object([
        ("ID", text(&shipment.id)),
        (
            "FreightAllowanceCharge",
            shipment
                .freight_allowance_charge
                .as_ref()
                .and_then(|a| one(allowance_charge(a, money))),
        ),
    ])
}

// Payment

fn payment_means(means: &PaymentMeans) -> Json {
    Json::object([
        ("PaymentMeansCode", text(&means.code)),
        (
            "PayeeFinancialAccount",
            means
                .payee_account
                .as_deref()
                .and_then(|id| one(Json::object([("ID", text(id))]))),
        ),
    ])
}

fn prepaid(payment: &PrepaidPayment, money: Money<'_>) -> Json {
    Json::object([
        ("ID", optional(payment.id.as_deref())),
        ("PaidAmount", money.of(payment.amount)),
        ("PaidDate", optional(payment.date.as_deref())),
        ("PaidTime", optional(payment.time.as_deref())),
    ])
}

fn allowance_charge(allowance: &AllowanceCharge, money: Money<'_>) -> Json {
    Json::object([
        (
            "ChargeIndicator",
            Some(Json::value(Json::Bool(allowance.charge), &[])),
        ),
        (
            "AllowanceChargeReason",
            optional(allowance.reason.as_deref()),
        ),
        ("MultiplierFactorNumeric", number(allowance.multiplier)),
        ("Amount", money.of(allowance.amount)),
    ])
}

fn exchange_rate(rate: &ExchangeRate) -> Json {
    Json::object([
        ("SourceCurrencyCode", text(&rate.source)),
        ("TargetCurrencyCode", text(&rate.target)),
        ("CalculationRate", number(rate.rate)),
    ])
}

// Tax and totals

fn tax_total(total: &TaxTotal, money: Money<'_>, tax_money: Money<'_>) -> Json {
    Json::object([
        ("TaxAmount", tax_money.of(total.tax_amount)),
        (
            "TaxSubtotal",
            many(&total.subtotals, |s| tax_subtotal(s, money, tax_money)),
        ),
    ])
}

fn tax_subtotal(subtotal: &TaxSubtotal, money: Money<'_>, tax_money: Money<'_>) -> Json {
    let per_unit = subtotal.per_unit.as_ref();
    Json::object([
        ("TaxableAmount", money.of(subtotal.taxable_amount)),
        ("TaxAmount", tax_money.of(subtotal.tax_amount)),
        ("Percent", number(subtotal.percent)),
        (
            "BaseUnitMeasure",
            per_unit.map(|unit| {
                Json::value(
                    Json::number(unit.quantity),
                    &[("unitCode", Some(unit.unit_code.as_str()))],
                )
            }),
        ),
        (
            "PerUnitAmount",
            per_unit.and_then(|unit| tax_money.of(unit.amount)),
        ),
        (
            "TaxCategory",
            one(Json::object([
                ("ID", text(&subtotal.tax_type)),
                (
                    "TaxExemptionReason",
                    optional(subtotal.exemption_reason.as_deref()),
                ),
                ("TaxScheme", one(tax_scheme())),
            ])),
        ),
    ])
}

fn tax_scheme() -> Json {
    Json::object([(
        "ID",
        Some(Json::value(
            Json::str("OTH"),
            &[
                ("schemeID", Some("UN/ECE 5153")),
                ("schemeAgencyID", Some("6")),
            ],
        )),
    )])
}

fn totals(totals: &Totals, money: Money<'_>) -> Json {
    Json::object([
        (
            "LineExtensionAmount",
            money.of(totals.line_extension_amount),
        ),
        ("TaxExclusiveAmount", money.of(totals.tax_exclusive_amount)),
        ("TaxInclusiveAmount", money.of(totals.tax_inclusive_amount)),
        (
            "AllowanceTotalAmount",
            money.maybe(totals.allowance_total_amount),
        ),
        ("ChargeTotalAmount", money.maybe(totals.charge_total_amount)),
        (
            "PayableRoundingAmount",
            money.maybe(totals.payable_rounding_amount),
        ),
        ("PayableAmount", money.of(totals.payable_amount)),
    ])
}

// Lines

fn line(line: &Line, money: Money<'_>, tax_money: Money<'_>) -> Json {
    Json::object([
        ("ID", text(&line.id)),
        (
            "InvoicedQuantity",
            Some(Json::value(
                Json::number(line.quantity),
                &[("unitCode", line.unit_code.as_deref())],
            )),
        ),
        ("LineExtensionAmount", money.of(line.line_extension_amount)),
        (
            "AllowanceCharge",
            many(&line.allowance_charges, |a| allowance_charge(a, money)),
        ),
        (
            "TaxTotal",
            one(tax_total(&line.tax_total, money, tax_money)),
        ),
        (
            "Item",
            one(Json::object([
                (
                    "CommodityClassification",
                    many(&line.classifications, |(list_id, code)| {
                        Json::object([(
                            "ItemClassificationCode",
                            Some(Json::value(Json::str(code), &[("listID", Some(list_id))])),
                        )])
                    }),
                ),
                ("Description", optional(line.description.as_deref())),
                (
                    "OriginCountry",
                    line.origin_country
                        .as_deref()
                        .and_then(|code| one(Json::object([("IdentificationCode", text(code))]))),
                ),
            ])),
        ),
        (
            "Price",
            one(Json::object([("PriceAmount", money.of(line.unit_price))])),
        ),
        (
            "ItemPriceExtension",
            one(Json::object([("Amount", money.of(line.subtotal))])),
        ),
    ])
}
