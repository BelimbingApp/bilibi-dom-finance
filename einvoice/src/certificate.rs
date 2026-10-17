//! A company's signing certificate and private key, read from PEM text
//! (Bilimbi `Bilimbi.Finance.Einvoice.Certificate`).
//!
//! A company exports its certificate and its unencrypted private key (`BEGIN
//! PRIVATE KEY` or `BEGIN RSA PRIVATE KEY`) as PEM and stores both as
//! encrypted settings. PKCS#12 is not read, for parity with Bilimbi. Only RSA
//! keys are accepted: the authority's signature is RSA PKCS#1 v1.5 with
//! SHA-256.
//!
//! [`Certificate::parse`] returns the facts a signature and the setup page
//! need: the DER bytes, the issuer name in the form the authority compares,
//! the serial number, the validity window, and the subject attributes that
//! carry the company's tax and registration numbers.
use crate::authority::CertificateSubject;
use jiff::Timestamp;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};
use x509_parser::pem::Pem;
use x509_parser::prelude::{FromDer, X509Certificate, X509Name};

/// Why PEM text is not a usable certificate or key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CertificateError {
    /// Not one readable X.509 certificate with an RSA public key.
    #[error("invalid certificate")]
    InvalidCertificate,
    /// Not exactly one unencrypted RSA private key.
    #[error("invalid private key")]
    InvalidPrivateKey,
}

/// The facts of one X.509 certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Certificate {
    der: Vec<u8>,
    public_key: RsaPublicKey,
    /// The issuer's distinguished name as the authority writes it: the
    /// attributes in reverse DER order, comma-joined, as .NET renders an
    /// `X500DistinguishedName`.
    pub issuer_name: String,
    /// The subject's distinguished name in the same form.
    pub subject_name: String,
    /// The serial number in decimal.
    pub serial_number: String,
    pub not_before: Timestamp,
    pub not_after: Timestamp,
    /// The subject attributes that carry the company's tax and registration
    /// numbers, as the authority writes them there.
    pub subject: CertificateSubject,
}

/// An unencrypted RSA private key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrivateKey(RsaPrivateKey);

// Short names in the form .NET's X500DistinguishedName renders, which is what
// the authority's samples carry.
const SHORT_NAMES: [(&str, &str); 13] = [
    ("2.5.4.3", "CN"),
    ("2.5.4.4", "SN"),
    ("2.5.4.5", "SERIALNUMBER"),
    ("2.5.4.6", "C"),
    ("2.5.4.7", "L"),
    ("2.5.4.8", "S"),
    ("2.5.4.9", "STREET"),
    ("2.5.4.10", "O"),
    ("2.5.4.11", "OU"),
    ("2.5.4.12", "T"),
    ("2.5.4.42", "G"),
    ("1.2.840.113549.1.9.1", "E"),
    ("0.9.2342.19200300.100.1.25", "DC"),
];
const ORGANIZATION_IDENTIFIER: &str = "2.5.4.97";
const SERIAL_NUMBER: &str = "2.5.4.5";

impl Certificate {
    /// Reads the first certificate in PEM text (entries after it are ignored).
    pub fn parse(pem: &str) -> Result<Certificate, CertificateError> {
        match Pem::iter_from_buffer(pem.as_bytes()).next() {
            Some(Ok(entry)) if entry.label == "CERTIFICATE" => Self::from_der(&entry.contents),
            _ => Err(CertificateError::InvalidCertificate),
        }
    }

    /// Reads a certificate's DER bytes.
    pub fn from_der(der: &[u8]) -> Result<Certificate, CertificateError> {
        let (_, certificate) =
            X509Certificate::from_der(der).map_err(|_| CertificateError::InvalidCertificate)?;
        let tbs = &certificate.tbs_certificate;
        let public_key = RsaPublicKey::from_public_key_der(tbs.subject_pki.raw)
            .map_err(|_| CertificateError::InvalidCertificate)?;
        let time = |seconds: i64| {
            Timestamp::from_second(seconds).map_err(|_| CertificateError::InvalidCertificate)
        };
        Ok(Certificate {
            der: der.to_vec(),
            public_key,
            issuer_name: distinguished_name(&tbs.issuer),
            subject_name: distinguished_name(&tbs.subject),
            serial_number: tbs.serial.to_string(),
            not_before: time(tbs.validity.not_before.timestamp())?,
            not_after: time(tbs.validity.not_after.timestamp())?,
            subject: subject_attributes(&tbs.subject),
        })
    }

    /// The certificate's DER bytes, as the signature embeds and digests them.
    pub fn der(&self) -> &[u8] {
        &self.der
    }

    pub(crate) fn public_key(&self) -> &RsaPublicKey {
        &self.public_key
    }

    /// True when `key` belongs to this certificate's public key.
    pub fn key_matches(&self, key: &PrivateKey) -> bool {
        key.0.to_public_key() == self.public_key
    }

    /// Whole days from `now` until the certificate stops being valid;
    /// negative once expired.
    pub fn days_until_expiry(&self, now: Timestamp) -> i64 {
        (self.not_after.as_second() - now.as_second()).div_euclid(86_400)
    }
}

impl PrivateKey {
    /// Reads exactly one unencrypted RSA private key from PEM text.
    pub fn parse(pem: &str) -> Result<PrivateKey, CertificateError> {
        let entries: Result<Vec<Pem>, _> = Pem::iter_from_buffer(pem.as_bytes()).collect();
        let Ok(entries) = entries else {
            return Err(CertificateError::InvalidPrivateKey);
        };
        let key = match entries.as_slice() {
            [entry] => match entry.label.as_str() {
                "PRIVATE KEY" => RsaPrivateKey::from_pkcs8_der(&entry.contents).ok(),
                "RSA PRIVATE KEY" => RsaPrivateKey::from_pkcs1_der(&entry.contents).ok(),
                _ => None,
            },
            _ => None,
        };
        key.map(PrivateKey)
            .ok_or(CertificateError::InvalidPrivateKey)
    }

    pub(crate) fn rsa(&self) -> &RsaPrivateKey {
        &self.0
    }
}

/// Settings validator: true when `pem` holds one readable X.509 certificate.
pub(crate) fn valid_certificate_pem(pem: &str) -> bool {
    Certificate::parse(pem).is_ok()
}

/// Settings validator: true when `pem` holds exactly one unencrypted RSA
/// private key.
pub(crate) fn valid_private_key_pem(pem: &str) -> bool {
    PrivateKey::parse(pem).is_ok()
}

fn distinguished_name(name: &X509Name<'_>) -> String {
    let rdns: Vec<String> = name
        .iter()
        .map(|rdn| {
            rdn.iter()
                .map(|attribute| {
                    format!(
                        "{}={}",
                        short_name(&attribute.attr_type().to_id_string()),
                        quote_value(&attribute_text(attribute.attr_value()))
                    )
                })
                .collect::<Vec<_>>()
                .join(" + ")
        })
        .collect();
    rdns.into_iter().rev().collect::<Vec<_>>().join(", ")
}

fn short_name(oid: &str) -> String {
    SHORT_NAMES
        .iter()
        .find(|(id, _)| *id == oid)
        .map(|(_, name)| (*name).to_owned())
        .unwrap_or_else(|| format!("OID.{oid}"))
}

fn quote_value(value: &str) -> String {
    let special = value.contains([',', '+', '=', '"', '\n', '<', '>', '#', ';'])
        || value.starts_with(char::is_whitespace)
        || value.ends_with(char::is_whitespace);
    if special {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

// The attribute's text by its string type: UTF8String, PrintableString,
// IA5String and T61String as the bytes they are, BMPString as UTF-16.
fn attribute_text(value: &x509_parser::der_parser::asn1_rs::Any<'_>) -> String {
    match value.tag().0 {
        0x1E => {
            let units: Vec<u16> = value
                .data
                .chunks(2)
                .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]))
                .collect();
            String::from_utf16_lossy(&units)
        }
        _ => String::from_utf8_lossy(value.data).into_owned(),
    }
}

// The tax and registration numbers an authority-issued certificate carries.
fn subject_attributes(name: &X509Name<'_>) -> CertificateSubject {
    let mut subject = CertificateSubject::default();
    for attribute in name.iter_attributes() {
        let oid = attribute.attr_type().to_id_string();
        if oid == ORGANIZATION_IDENTIFIER {
            subject.organization_identifier = Some(attribute_text(attribute.attr_value()));
        } else if oid == SERIAL_NUMBER {
            subject.serial_number = Some(attribute_text(attribute.attr_value()));
        }
    }
    subject
}
