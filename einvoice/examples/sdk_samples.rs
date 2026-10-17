//! Rebuilds the MyInvois SDK's four version 1.1 JSON invoice samples byte for
//! byte, verifies their signatures, and signs with a throwaway certificate.
//!
//! The samples (`examples/sdk/`, from
//! <https://sdk.myinvois.hasil.gov.my/documents/invoice-v1-1/>, read on 12
//! October 2026) are signed, and each signature's document digest is the
//! SHA-256 of the unsigned, minified bytes. Rebuilding those bytes from plain
//! builder input proves the builder writes what the authority hashed. The
//! values are the SDK's placeholders; the figures are the SDK's and do not
//! all add up, which is why the builder maps without computing. The signing
//! certificate is the SDK's dummy test certificate.
//!
//! The throwaway certificate and key are made with `openssl` in a temporary
//! directory that is removed on return. Exit status 1 when any check fails.
//!
//! ```sh
//! cargo run -p bilibi-finance-einvoice --example sdk_samples
//! ```
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use bilibi_finance_einvoice::myinvois::json::Json;
use bilibi_finance_einvoice::myinvois::signer::{self, SigningKey, VerifyError};
use bilibi_finance_einvoice::myinvois::ubl::{
    self, AdditionalAccount, Address, AllowanceCharge, BillingReference, Delivery, Document,
    DocumentReference, Line, Party, PaymentMeans, PerUnit, Period, PrepaidPayment, Shipment,
    TaxSubtotal, TaxTotal, Totals,
};
use bilibi_finance_einvoice::{Certificate, PrivateKey};
use jiff::Timestamp;
use rust_decimal::Decimal;
use std::process::Command;

const SAMPLES: [(&str, &str); 4] = [
    ("Invoice", include_str!("sdk/1.1-Invoice-Sample.json")),
    (
        "MultiLineItem",
        include_str!("sdk/1.1-Invoice-MultiLineItem-Sample.json"),
    ),
    (
        "ForeignCurrency",
        include_str!("sdk/1.1-Invoice-ForeignCurrency-Sample.json"),
    ),
    (
        "Consolidated",
        include_str!("sdk/1.1-Invoice-Consolidated-Sample.json"),
    ),
];
const SIGNATURE_PATH: [&str; 7] = [
    "Invoice",
    "UBLExtensions",
    "UBLExtension",
    "ExtensionContent",
    "UBLDocumentSignatures",
    "SignatureInformation",
    "Signature",
];

struct Checks {
    failed: usize,
}

impl Checks {
    fn check(&mut self, what: &str, ok: bool) {
        println!("{}: {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.failed += 1;
        }
    }
}

fn main() {
    let mut checks = Checks { failed: 0 };

    println!("== The SDK's samples, rebuilt byte for byte");
    for (name, signed) in SAMPLES {
        let sample = Json::decode(signed).expect("the sample is JSON");
        let built = ubl::build(&input(name));
        let sample_bytes = signer::document_bytes(&sample);
        let built_bytes = built.encode();
        checks.check(
            &format!(
                "{name}: {} unsigned bytes rebuilt from plain input",
                sample_bytes.len()
            ),
            built_bytes.as_bytes() == sample_bytes,
        );
        if built_bytes.as_bytes() != sample_bytes {
            let at = built_bytes
                .bytes()
                .zip(sample_bytes.iter())
                .position(|(a, b)| a != *b)
                .unwrap_or(sample_bytes.len().min(built_bytes.len()));
            let window = |bytes: &[u8]| {
                String::from_utf8_lossy(&bytes[at.saturating_sub(60)..(at + 60).min(bytes.len())])
                    .into_owned()
            };
            println!("  first difference at byte {at}");
            println!("  built:  ...{}...", window(built_bytes.as_bytes()));
            println!("  sample: ...{}...", window(&sample_bytes));
        }
        let sample_digest = signer::document_digest(&sample);
        checks.check(
            &format!("{name}: document digest {sample_digest}"),
            signer::document_digest(&built) == sample_digest,
        );
        let digest_in_signature = sample
            .at(&SIGNATURE_PATH)
            .and_then(|signature| signature.first("SignedInfo"))
            .and_then(|info| match info.get("Reference") {
                Some(Json::Array(references)) => references.get(1),
                _ => None,
            })
            .and_then(|reference| reference.text_at(&["DigestValue"]))
            .map(str::to_owned);
        checks.check(
            &format!("{name}: the digest the SDK's signature carries is that digest"),
            digest_in_signature.as_deref() == Some(sample_digest.as_str()),
        );
    }

    println!("\n== Leaving out what the input does not have");
    let mut reduced = input("Invoice");
    reduced.delivery = None;
    reduced.invoice_period = None;
    reduced.payment_means = None;
    let bytes = ubl::build(&reduced).encode();
    checks.check(
        "no Delivery, InvoicePeriod or PaymentMeans",
        !bytes.contains("Delivery")
            && !bytes.contains("InvoicePeriod")
            && !bytes.contains("PaymentMeans"),
    );
    checks.check(
        "the type code carries the format version",
        bytes.contains(r#""InvoiceTypeCode":[{"_":"01","listVersionID":"1.1"}]"#),
    );
    let mut note = input("Invoice");
    note.type_code = "02".into();
    note.billing_references = vec![BillingReference {
        id: "INV-1".into(),
        uuid: Some("F9D425P6DS7D8IU".into()),
    }];
    checks.check(
        "a note references its original by ID and UUID",
        ubl::build(&note).encode().contains(
            r#""BillingReference":[{"InvoiceDocumentReference":[{"ID":[{"_":"INV-1"}],"UUID":[{"_":"F9D425P6DS7D8IU"}]}]}]"#,
        ),
    );

    println!("\n== The SDK's signatures verify");
    for (name, signed) in SAMPLES {
        let outcome = signer::verify(signed.as_bytes());
        checks.check(
            &format!("{name}: digests, certificate, issuer and RSA signature ({outcome:?})"),
            outcome.is_ok(),
        );
    }
    let sample = Json::decode(SAMPLES[0].1).expect("the sample is JSON");
    let signature = sample.at(&SIGNATURE_PATH).expect("the sample is signed");
    let der = BASE64
        .decode(
            signature
                .text_at(&["KeyInfo", "X509Data", "X509Certificate"])
                .expect("the signature embeds its certificate"),
        )
        .expect("the certificate is base64");
    let sdk_certificate = Certificate::from_der(&der).expect("the SDK certificate reads");
    println!("SDK certificate issuer:  {}", sdk_certificate.issuer_name);
    println!("SDK certificate subject: {}", sdk_certificate.subject_name);
    println!("SDK certificate serial:  {}", sdk_certificate.serial_number);
    println!(
        "SDK certificate valid:   {} to {}",
        sdk_certificate.not_before, sdk_certificate.not_after
    );
    println!(
        "SDK certificate subject attributes: {:?}",
        sdk_certificate.subject
    );
    checks.check(
        "the issuer name is .NET's form: attributes in reverse DER order",
        sdk_certificate.issuer_name
            == "C=MY, O=Raffcomm Technologies Sdn Bhd, OU=1000449-W, CN=CypherSign Pro Max",
    );
    checks.check(
        "the serial number is decimal",
        sdk_certificate.serial_number == "1308502606566147853",
    );
    let properties = signature
        .at(&["Object", "QualifyingProperties"])
        .expect("the signature carries its properties");
    let rebuilt = signer::qualifying_properties(
        &sdk_certificate,
        "2025-04-15T02:01:36Z".parse().expect("a timestamp"),
    );
    checks.check(
        "signed properties rebuilt from the SDK certificate match the sample's",
        rebuilt.encode() == properties.encode(),
    );

    println!("\n== A changed document or signature no longer verifies");
    let tampered = SAMPLES[0]
        .1
        .replace("Laptop Peripherals", "Laptop Peripheralz");
    let outcome = signer::verify(tampered.as_bytes());
    checks.check(
        &format!("a changed description: {outcome:?}"),
        outcome == Err(VerifyError::DocumentDigestMismatch),
    );
    let value = signature
        .text_at(&["SignatureValue"])
        .expect("the signature has a value");
    let forged = BASE64.encode(vec![0x42u8; BASE64.decode(value).expect("base64").len()]);
    let outcome = signer::verify(SAMPLES[0].1.replace(value, &forged).as_bytes());
    checks.check(
        &format!("a forged signature value: {outcome:?}"),
        outcome == Err(VerifyError::BadSignature),
    );

    println!("\n== Signing with a throwaway certificate");
    match throwaway() {
        Ok((signing_key, other_key)) => {
            let built = ubl::build(&input("Invoice"));
            let signing_time: Timestamp =
                "2026-10-12T08:00:00.123456Z".parse().expect("a timestamp");
            let signed = signer::sign(&built, &signing_key, signing_time).expect("signs");
            let outcome = signer::verify(&signed);
            checks.check(
                &format!("the signed document verifies ({outcome:?})"),
                outcome.is_ok(),
            );
            let text = String::from_utf8_lossy(&signed);
            checks.check(
                "the signing time is truncated to the second, UTC",
                text.contains(r#""SigningTime":[{"_":"2026-10-12T08:00:00Z"}]"#),
            );
            let signed_json = Json::decode(&text).expect("the signed document is JSON");
            checks.check(
                "the signed document keeps the SDK sample's shape",
                shape(&signed_json) == shape(&sample),
            );
            checks.check(
                "its unsigned bytes are the sample's",
                signer::document_bytes(&signed_json) == signer::document_bytes(&sample),
            );
            let resigned = signer::sign(&sample, &signing_key, signing_time).expect("signs");
            let text = String::from_utf8_lossy(&resigned);
            checks.check(
                &format!(
                    "re-signing replaces an existing signature ({:?})",
                    signer::verify(&resigned)
                ),
                signer::verify(&resigned).is_ok() && text.matches("UBLExtensions").count() == 1,
            );
            let wrong = SigningKey {
                certificate: signing_key.certificate.clone(),
                key: other_key,
            };
            let outcome =
                signer::verify(&signer::sign(&built, &wrong, signing_time).expect("signs"));
            checks.check(
                &format!("a key that is not the certificate's fails verification: {outcome:?}"),
                outcome == Err(VerifyError::BadSignature),
            );
            checks.check(
                "the certificate knows its own key",
                signing_key.certificate.key_matches(&signing_key.key)
                    && !signing_key.certificate.key_matches(&wrong.key),
            );
        }
        Err(error) => checks.check(
            &format!("openssl made a throwaway certificate ({error})"),
            false,
        ),
    }

    println!();
    if checks.failed == 0 {
        println!("all checks passed");
    } else {
        println!("{} check(s) FAILED", checks.failed);
        std::process::exit(1);
    }
}

/// The structure with every value blanked: keys, order and nesting only.
fn shape(json: &Json) -> String {
    match json {
        Json::Object(pairs) => format!(
            "{{{}}}",
            pairs
                .iter()
                .map(|(key, value)| format!("{key}:{}", shape(value)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Json::Array(items) => format!(
            "[{}]",
            items.iter().map(shape).collect::<Vec<_>>().join(",")
        ),
        _ => "_".into(),
    }
}

/// A throwaway RSA certificate and key, and a second key that is not the
/// certificate's, as PEM files `openssl` writes into a temporary directory
/// that is removed on return.
fn throwaway() -> Result<(SigningKey, PrivateKey), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("einvoice-sdk-samples-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let run = |args: &[&str]| -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("openssl").args(args).output()?.status;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "openssl failed".into())
    };
    let (cert, key, other) = (path("cert.pem"), path("key.pem"), path("other.pem"));
    run(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "30",
        "-subj",
        "/C=MY/O=Supplier's Name/organizationIdentifier=C29702635060/CN=Supplier's Name/serialNumber=D12345678",
        "-keyout",
        &key,
        "-out",
        &cert,
    ])?;
    run(&["genrsa", "-out", &other, "2048"])?;
    let read = |file: &str| std::fs::read_to_string(file);
    let signing_key = SigningKey {
        certificate: Certificate::parse(&read(&cert)?)?,
        key: PrivateKey::parse(&read(&key)?)?,
    };
    let other_key = PrivateKey::parse(&read(&other)?)?;
    std::fs::remove_dir_all(&dir)?;
    println!(
        "throwaway certificate subject: {}",
        signing_key.certificate.subject_name
    );
    println!(
        "throwaway certificate subject attributes: {:?}",
        signing_key.certificate.subject
    );
    Ok((signing_key, other_key))
}

// Builder input that reproduces each sample.

fn d(text: &str) -> Decimal {
    Decimal::from_str_exact(text).expect("a decimal")
}

fn s(text: &str) -> String {
    text.to_owned()
}

fn address() -> Address {
    Address {
        city: s("Kuala Lumpur"),
        postcode: Some(s("50480")),
        state_code: s("10"),
        lines: vec![s("Lot 66"), s("Bangunan Merdeka"), s("Persiaran Jaya")],
        country_code: s("MYS"),
    }
}

fn identifications(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs.iter().map(|(a, b)| (s(a), s(b))).collect()
}

fn supplier(additional_account: bool) -> Party {
    Party {
        additional_account: additional_account.then(|| AdditionalAccount {
            id: s("CPT-CCN-W-211111-KL-000002"),
            scheme_agency_name: s("CertEX"),
        }),
        industry_code: Some((
            s("46510"),
            s("Wholesale of computer hardware, software and peripherals"),
        )),
        identifications: identifications(&[
            ("TIN", "Supplier's TIN"),
            ("BRN", "Supplier's BRN"),
            ("SST", "NA"),
            ("TTX", "NA"),
        ]),
        address: address(),
        name: s("Supplier's Name"),
        phone: Some(s("+60123456789")),
        email: Some(s("supplier@email.com")),
    }
}

fn buyer() -> Party {
    Party {
        address: address(),
        name: s("Buyer's Name"),
        identifications: identifications(&[
            ("TIN", "Buyer's TIN"),
            ("BRN", "Buyer's BRN"),
            ("SST", "NA"),
            ("TTX", "NA"),
        ]),
        phone: Some(s("+60123456789")),
        email: Some(s("buyer@email.com")),
        industry_code: None,
        additional_account: None,
    }
}

fn allowance(
    charge: bool,
    reason: &str,
    multiplier: Option<&str>,
    amount: &str,
) -> AllowanceCharge {
    AllowanceCharge {
        charge,
        reason: Some(s(reason)),
        multiplier: multiplier.map(d),
        amount: d(amount),
    }
}

fn subtotal(taxable: &str, tax: &str, tax_type: &str) -> TaxSubtotal {
    TaxSubtotal {
        taxable_amount: d(taxable),
        tax_amount: d(tax),
        percent: None,
        per_unit: None,
        tax_type: s(tax_type),
        exemption_reason: None,
    }
}

fn input(name: &str) -> Document {
    match name {
        "Invoice" => invoice(),
        "ForeignCurrency" => {
            let mut document = invoice();
            document.currency_code = s("USD");
            document.tax_exchange_rate = Some(ubl::ExchangeRate {
                source: s("USD"),
                target: s("MYR"),
                rate: Some(d("4.72")),
            });
            document
        }
        "MultiLineItem" => multi_line_item(),
        "Consolidated" => consolidated(),
        other => panic!("no sample named {other}"),
    }
}

fn invoice() -> Document {
    Document {
        id: s("JSON-INV12345"),
        issue_date: s("2024-07-23"),
        issue_time: s("00:30:00Z"),
        type_code: s("01"),
        version: s("1.1"),
        currency_code: s("MYR"),
        tax_currency_code: Some(s("MYR")),
        tax_exchange_rate: None,
        invoice_period: Some(Period {
            start_date: Some(s("2024-01-01")),
            end_date: Some(s("2024-07-31")),
            description: Some(s("Monthly")),
        }),
        billing_references: vec![BillingReference {
            id: s("E12345678912"),
            uuid: None,
        }],
        additional_document_references: vec![
            DocumentReference {
                id: s("E12345678912"),
                document_type: Some(s("CustomsImportForm")),
                description: None,
            },
            DocumentReference {
                id: s("sa313321312"),
                document_type: Some(s("213312dddddd")),
                description: Some(s("asddasdwqfd ddq")),
            },
            DocumentReference {
                id: s("E12345678912"),
                document_type: Some(s("K2")),
                description: None,
            },
            DocumentReference {
                id: s("CIF"),
                document_type: None,
                description: None,
            },
        ],
        supplier: supplier(true),
        buyer: buyer(),
        delivery: Some(Delivery {
            party: Some(Party {
                name: s("Recipient's Name"),
                address: address(),
                identifications: identifications(&[
                    ("TIN", "Recipient's TIN"),
                    ("BRN", "Recipient's BRN"),
                ]),
                phone: None,
                email: None,
                industry_code: None,
                additional_account: None,
            }),
            shipment: Some(Shipment {
                id: s("1234"),
                freight_allowance_charge: Some(allowance(true, "Service charge", None, "100")),
            }),
        }),
        payment_means: Some(PaymentMeans {
            code: s("03"),
            payee_account: Some(s("1234567890123")),
        }),
        payment_terms: Some(s("Payment method is cash")),
        prepaid_payment: Some(PrepaidPayment {
            id: Some(s("E12345678912")),
            amount: d("1"),
            date: Some(s("2024-07-23")),
            time: Some(s("00:30:00Z")),
        }),
        allowance_charges: vec![
            allowance(false, "Sample Description", None, "100"),
            allowance(true, "Service charge", None, "100"),
        ],
        tax_total: TaxTotal {
            tax_amount: d("87.63"),
            subtotals: vec![subtotal("87.63", "87.63", "01")],
        },
        totals: Totals {
            line_extension_amount: d("1436.5"),
            tax_exclusive_amount: d("1436.5"),
            tax_inclusive_amount: d("1436.5"),
            allowance_total_amount: Some(d("1436.5")),
            charge_total_amount: Some(d("1436.5")),
            payable_rounding_amount: Some(d("0.3")),
            payable_amount: d("1436.5"),
        },
        lines: vec![Line {
            id: s("1234"),
            quantity: d("1"),
            unit_code: Some(s("C62")),
            line_extension_amount: d("1436.5"),
            allowance_charges: vec![
                allowance(false, "Sample Description", Some("0.15"), "100"),
                allowance(true, "Sample Description", Some("0.1"), "100"),
            ],
            tax_total: TaxTotal {
                tax_amount: d("1460.5"),
                subtotals: vec![TaxSubtotal {
                    taxable_amount: d("1460.5"),
                    tax_amount: d("1460.5"),
                    percent: Some(d("6")),
                    per_unit: None,
                    tax_type: s("E"),
                    exemption_reason: Some(s("Exempt New Means of Transport")),
                }],
            },
            classifications: identifications(&[("PTC", "9800.00.0010"), ("CLASS", "003")]),
            description: Some(s("Laptop Peripherals")),
            origin_country: Some(s("MYS")),
            unit_price: d("17"),
            subtotal: d("100"),
        }],
    }
}

fn multi_line_item() -> Document {
    let zero_allowances = || {
        vec![
            allowance(false, "", Some("0.0"), "0.0"),
            allowance(true, "", Some("0.0"), "0.0"),
        ]
    };
    let line = |id: &str,
                amount: &str,
                tariff: &str,
                description: &str,
                subtotals: Vec<TaxSubtotal>,
                tax: &str| Line {
        id: s(id),
        quantity: d("1"),
        unit_code: Some(s("C62")),
        line_extension_amount: d(amount),
        allowance_charges: zero_allowances(),
        tax_total: TaxTotal {
            tax_amount: d(tax),
            subtotals,
        },
        classifications: identifications(&[("PTC", tariff), ("CLASS", "003")]),
        description: Some(s(description)),
        origin_country: Some(s("MYS")),
        unit_price: d(amount),
        subtotal: d(amount),
    };
    let mut document = invoice();
    document.delivery = None;
    document.payment_means = None;
    document.payment_terms = None;
    document.prepaid_payment = None;
    document.issue_date = s("2025-02-06");
    document.invoice_period = Some(Period {
        start_date: Some(s("2025-01-01")),
        end_date: Some(s("2025-01-31")),
        description: Some(s("Monthly")),
    });
    document.additional_document_references = vec![
        DocumentReference {
            id: s("E23456789123,E98765432123"),
            document_type: Some(s("CustomsImportForm")),
            description: None,
        },
        DocumentReference {
            id: s("FTA"),
            document_type: Some(s("FreeTradeAgreement")),
            description: Some(s("ASEAN-Australia-New Zealand FTA (AANZFTA)")),
        },
        DocumentReference {
            id: s("E12345678912,E23456789123"),
            document_type: Some(s("K2")),
            description: None,
        },
        DocumentReference {
            id: s("CIF"),
            document_type: None,
            description: None,
        },
    ];
    document.allowance_charges = vec![
        allowance(false, "", None, "0.0"),
        allowance(true, "", None, "0.0"),
    ];
    document.tax_total = TaxTotal {
        tax_amount: d("85.0"),
        subtotals: vec![TaxSubtotal {
            taxable_amount: d("2000.0"),
            tax_amount: d("100.0"),
            percent: None,
            per_unit: None,
            tax_type: s("E"),
            exemption_reason: Some(s("Special case")),
        }],
    };
    document.totals = Totals {
        line_extension_amount: d("4500.0"),
        tax_exclusive_amount: d("4500.0"),
        tax_inclusive_amount: d("4585.0"),
        allowance_total_amount: Some(d("0.0")),
        charge_total_amount: Some(d("0.0")),
        payable_rounding_amount: Some(d("0.0")),
        payable_amount: d("4585.0"),
    };
    document.lines = vec![
        line(
            "001",
            "1000.0",
            "9800.00.0010",
            "Computer Monitor",
            vec![TaxSubtotal {
                taxable_amount: d("1000.0"),
                tax_amount: d("10.0"),
                percent: None,
                per_unit: Some(PerUnit {
                    quantity: d("1"),
                    unit_code: s("C62"),
                    amount: d("10.0"),
                }),
                tax_type: s("01"),
                exemption_reason: None,
            }],
            "10.0",
        ),
        line(
            "002",
            "1500.0",
            "9800.00.0011",
            "Laptop Peripherals",
            vec![TaxSubtotal {
                taxable_amount: d("1500.0"),
                tax_amount: d("75.0"),
                percent: Some(d("5.0")),
                per_unit: None,
                tax_type: s("02"),
                exemption_reason: None,
            }],
            "75.0",
        ),
        line(
            "003",
            "2000.0",
            "9800.00.0012",
            "Wireless Mouse",
            vec![
                TaxSubtotal {
                    taxable_amount: d("0.0"),
                    tax_amount: d("0.0"),
                    percent: Some(d("5.0")),
                    per_unit: None,
                    tax_type: s("01"),
                    exemption_reason: None,
                },
                TaxSubtotal {
                    taxable_amount: d("2000.0"),
                    tax_amount: d("100.0"),
                    percent: None,
                    per_unit: None,
                    tax_type: s("E"),
                    exemption_reason: Some(s("Special Case")),
                },
            ],
            "0.0",
        ),
    ];
    document
}

fn consolidated() -> Document {
    let line = |id: &str, amount: &str, tax: &str, description: &str| Line {
        id: s(id),
        quantity: d("1"),
        unit_code: Some(s("C62")),
        line_extension_amount: d(amount),
        allowance_charges: Vec::new(),
        tax_total: TaxTotal {
            tax_amount: d(tax),
            subtotals: vec![TaxSubtotal {
                taxable_amount: d(amount),
                tax_amount: d(tax),
                percent: Some(d("10")),
                per_unit: None,
                tax_type: s("01"),
                exemption_reason: None,
            }],
        },
        classifications: identifications(&[("CLASS", "004")]),
        description: Some(s(description)),
        origin_country: Some(s("MYS")),
        unit_price: d(amount),
        subtotal: d(amount),
    };
    Document {
        id: s("JSON-INV12345"),
        issue_date: s("2024-07-23"),
        issue_time: s("00:30:00Z"),
        type_code: s("01"),
        version: s("1.1"),
        currency_code: s("MYR"),
        tax_currency_code: Some(s("MYR")),
        tax_exchange_rate: None,
        invoice_period: None,
        billing_references: Vec::new(),
        additional_document_references: Vec::new(),
        supplier: supplier(false),
        buyer: Party {
            address: Address {
                city: s(""),
                postcode: Some(s("")),
                state_code: s(""),
                lines: vec![s("NA"), s(""), s("")],
                country_code: s(""),
            },
            name: s("Consolidated Buyer's"),
            identifications: identifications(&[
                ("TIN", "EI00000000010"),
                ("BRN", "NA"),
                ("SST", "NA"),
                ("TTX", "NA"),
            ]),
            phone: Some(s("NA")),
            email: Some(s("NA")),
            industry_code: None,
            additional_account: None,
        },
        delivery: None,
        payment_means: None,
        payment_terms: None,
        prepaid_payment: None,
        allowance_charges: Vec::new(),
        tax_total: TaxTotal {
            tax_amount: d("3000"),
            subtotals: vec![subtotal("30000", "3000", "01")],
        },
        totals: Totals {
            line_extension_amount: d("30000"),
            tax_exclusive_amount: d("30000"),
            tax_inclusive_amount: d("33000"),
            allowance_total_amount: None,
            charge_total_amount: None,
            payable_rounding_amount: None,
            payable_amount: d("33000"),
        },
        lines: vec![
            line("1", "10000", "1000", "Receipt 001 - 100"),
            line("2", "20000", "2000", "Receipt 101 - 200"),
        ],
    }
}
