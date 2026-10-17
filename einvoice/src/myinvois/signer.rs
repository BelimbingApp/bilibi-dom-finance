//! Enveloped XAdES signatures on UBL JSON documents, as the MyInvois SDK's
//! "Signing a JSON document" page describes them (Bilimbi
//! `Bilimbi.Finance.Einvoice.MyInvois.Signer`).
//!
//! 1. Remove `UBLExtensions` and `Signature` from the invoice and minify the
//!    document; these bytes are what the authority hashes.
//! 2. The document digest is their SHA-256, base64-encoded.
//! 3. The signature is RSA PKCS#1 v1.5 with SHA-256 over those bytes.
//! 4. The certificate digest is the SHA-256 of the certificate's DER bytes.
//! 5. `SignedProperties` carry the UTC signing time, the certificate digest,
//!    and the issuer name and serial number.
//! 6. Their digest is the SHA-256 of the minified
//!    `{"Target": "signature", "SignedProperties": [...]}` object.
//! 7. `UBLExtensions` and `Signature` are appended to the invoice, and the
//!    whole document is minified again.
//!
//! JSON needs no XML canonicalisation; the bytes are the canonical form,
//! which is why the order-preserving writer in `json.rs` writes them. The
//! `sdk_samples` example verifies the SDK's signed samples with this module
//! and signs with a generated certificate.
use super::json::{Json, JsonError};
use crate::certificate::{Certificate, PrivateKey};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use jiff::Timestamp;
use rsa::pkcs1v15::Pkcs1v15Sign;
use rsa::sha2::{Digest, Sha256};

const SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
const RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
const XADES: &str = "urn:oasis:names:specification:ubl:dsig:enveloped:xades";
const SIGNATURE_ID: &str = "urn:oasis:names:specification:ubl:signature:Invoice";
const SIGNED_PROPERTIES_TYPE: &str = "http://uri.etsi.org/01903/v1.3.2#SignedProperties";
const SIGNATURE_PATH: [&str; 7] = [
    "Invoice",
    "UBLExtensions",
    "UBLExtension",
    "ExtensionContent",
    "UBLDocumentSignatures",
    "SignatureInformation",
    "Signature",
];

/// A company's certificate and the private key that belongs to it.
#[derive(Clone, Debug)]
pub struct SigningKey {
    pub certificate: Certificate,
    pub key: PrivateKey,
}

/// Why a signed document does not verify.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    #[error(transparent)]
    InvalidJson(#[from] JsonError),
    /// No signature at the place the SDK puts it.
    #[error("unsigned")]
    Unsigned,
    #[error("invalid base64")]
    InvalidBase64,
    /// The embedded certificate does not read.
    #[error("invalid certificate")]
    InvalidCertificate,
    /// The document's bytes are not what the signature's digest hashed.
    #[error("document digest mismatch")]
    DocumentDigestMismatch,
    #[error("signed properties digest mismatch")]
    SignedPropertiesDigestMismatch,
    #[error("certificate digest mismatch")]
    CertificateDigestMismatch,
    /// The signed properties name another issuer or serial than the
    /// embedded certificate's.
    #[error("issuer serial mismatch")]
    IssuerSerialMismatch,
    /// The RSA signature is not the certificate's over the document.
    #[error("bad signature")]
    BadSignature,
}

/// Why signing did not happen: the key refused to sign (a malformed key).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the private key did not sign")]
pub struct SignError;

/// The signed document's minified bytes. `document` is a UBL JSON term from
/// the builder or [`Json::decode`]; any signature it already carries is
/// replaced. `signing_time` is truncated to the second.
pub fn sign(
    document: &Json,
    signing_key: &SigningKey,
    signing_time: Timestamp,
) -> Result<Vec<u8>, SignError> {
    let unsigned = unsigned(document);
    let bytes = unsigned.encode();
    let signature = signing_key
        .key
        .rsa()
        .sign(
            Pkcs1v15Sign::new::<Sha256>(),
            &Sha256::digest(bytes.as_bytes()),
        )
        .map_err(|_| SignError)?;
    let properties = signed_properties(&signing_key.certificate, signing_time);
    let certificate = &signing_key.certificate;

    let extensions = Json::Array(vec![Json::object([(
        "UBLExtension",
        one(Json::object([
            ("ExtensionURI", Json::text(Some(XADES))),
            (
                "ExtensionContent",
                one(Json::object([(
                    "UBLDocumentSignatures",
                    one(Json::object([(
                        "SignatureInformation",
                        one(Json::object([
                            (
                                "ID",
                                Json::text(Some("urn:oasis:names:specification:ubl:signature:1")),
                            ),
                            ("ReferencedSignatureID", Json::text(Some(SIGNATURE_ID))),
                            (
                                "Signature",
                                one(signature_block(
                                    certificate,
                                    &properties,
                                    &signature,
                                    bytes.as_bytes(),
                                )),
                            ),
                        ])),
                    )])),
                )])),
            ),
        ])),
    )])]);
    let reference = Json::Array(vec![Json::object([
        ("ID", Json::text(Some(SIGNATURE_ID))),
        ("SignatureMethod", Json::text(Some(XADES))),
    ])]);

    let signed = unsigned.update_first("Invoice", |invoice| {
        invoice.with(vec![
            ("UBLExtensions", extensions),
            ("Signature", reference),
        ])
    });
    Ok(signed.encode().into_bytes())
}

/// The document's bytes as the authority hashes them: unsigned and minified.
pub fn document_bytes(document: &Json) -> Vec<u8> {
    unsigned(document).encode().into_bytes()
}

/// The base64 SHA-256 the authority calls the document digest.
pub fn document_digest(document: &Json) -> String {
    digest(&document_bytes(document))
}

/// The `QualifyingProperties` object a signature carries for a certificate
/// and signing time. Its minified form is what the signed-properties digest
/// hashes.
pub fn qualifying_properties(certificate: &Certificate, signing_time: Timestamp) -> Json {
    signed_properties(certificate, signing_time)
}

/// Checks a signed document's signature against the certificate it embeds:
/// the document digest, the signed-properties digest, the certificate digest,
/// the issuer and serial number, and the RSA signature over the document.
pub fn verify(signed: &[u8]) -> Result<(), VerifyError> {
    let text = std::str::from_utf8(signed).map_err(|_| JsonError {
        at: 0,
        what: "not UTF-8",
    })?;
    let document = Json::decode(text)?;
    let signature = document.at(&SIGNATURE_PATH).ok_or(VerifyError::Unsigned)?;
    let der = decode64(signature.text_at(&["KeyInfo", "X509Data", "X509Certificate"]))?;
    let certificate = Certificate::from_der(&der).map_err(|_| VerifyError::InvalidCertificate)?;
    let properties = signature
        .at(&["Object", "QualifyingProperties"])
        .ok_or(VerifyError::Unsigned)?;
    check_digests(signature, &document, properties)?;
    check_certificate(properties, &certificate)?;
    let value = decode64(signature.text_at(&["SignatureValue"]))?;
    certificate
        .public_key()
        .verify(
            Pkcs1v15Sign::new::<Sha256>(),
            &Sha256::digest(document_bytes(&document)),
            &value,
        )
        .map_err(|_| VerifyError::BadSignature)
}

// Signing

fn one(item: Json) -> Option<Json> {
    Some(Json::Array(vec![item]))
}

fn unsigned(document: &Json) -> Json {
    document.update_first("Invoice", |invoice| {
        invoice.without(&["UBLExtensions", "Signature"])
    })
}

fn signed_properties(certificate: &Certificate, signing_time: Timestamp) -> Json {
    let seconds = Timestamp::from_second(signing_time.as_second()).unwrap_or(signing_time);
    let time = seconds.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    Json::object([
        ("Target", Some(Json::str("signature"))),
        (
            "SignedProperties",
            one(Json::object([
                ("Id", Some(Json::str("id-xades-signed-props"))),
                (
                    "SignedSignatureProperties",
                    one(Json::object([
                        ("SigningTime", Json::text(Some(&time))),
                        (
                            "SigningCertificate",
                            one(Json::object([(
                                "Cert",
                                one(Json::object([
                                    (
                                        "CertDigest",
                                        one(Json::object([
                                            ("DigestMethod", digest_method()),
                                            (
                                                "DigestValue",
                                                Json::text(Some(&digest(certificate.der()))),
                                            ),
                                        ])),
                                    ),
                                    ("IssuerSerial", issuer_serial(certificate)),
                                ])),
                            )])),
                        ),
                    ])),
                ),
            ])),
        ),
    ])
}

fn signature_block(
    certificate: &Certificate,
    properties: &Json,
    signature: &[u8],
    bytes: &[u8],
) -> Json {
    Json::object([
        ("Id", Some(Json::str("signature"))),
        (
            "Object",
            one(Json::object([(
                "QualifyingProperties",
                one(properties.clone()),
            )])),
        ),
        (
            "KeyInfo",
            one(Json::object([(
                "X509Data",
                one(Json::object([
                    (
                        "X509Certificate",
                        Json::text(Some(&BASE64.encode(certificate.der()))),
                    ),
                    // The authority's samples carry the issuer name here too.
                    (
                        "X509SubjectName",
                        Json::text(Some(&certificate.issuer_name)),
                    ),
                    ("X509IssuerSerial", issuer_serial(certificate)),
                ])),
            )])),
        ),
        (
            "SignatureValue",
            Json::text(Some(&BASE64.encode(signature))),
        ),
        (
            "SignedInfo",
            one(Json::object([
                (
                    "SignatureMethod",
                    Some(Json::value(
                        Json::str(""),
                        &[("Algorithm", Some(RSA_SHA256))],
                    )),
                ),
                (
                    "Reference",
                    Some(Json::Array(vec![
                        Json::object([
                            ("Type", Some(Json::str(SIGNED_PROPERTIES_TYPE))),
                            ("URI", Some(Json::str("#id-xades-signed-props"))),
                            ("DigestMethod", digest_method()),
                            (
                                "DigestValue",
                                Json::text(Some(&digest(properties.encode().as_bytes()))),
                            ),
                        ]),
                        Json::object([
                            ("Type", Some(Json::str(""))),
                            ("URI", Some(Json::str(""))),
                            ("DigestMethod", digest_method()),
                            ("DigestValue", Json::text(Some(&digest(bytes)))),
                        ]),
                    ])),
                ),
            ])),
        ),
    ])
}

fn issuer_serial(certificate: &Certificate) -> Option<Json> {
    one(Json::object([
        ("X509IssuerName", Json::text(Some(&certificate.issuer_name))),
        (
            "X509SerialNumber",
            Json::text(Some(&certificate.serial_number)),
        ),
    ]))
}

fn digest_method() -> Option<Json> {
    Some(Json::value(Json::str(""), &[("Algorithm", Some(SHA256))]))
}

fn digest(bytes: &[u8]) -> String {
    BASE64.encode(Sha256::digest(bytes))
}

// Verifying

fn check_digests(signature: &Json, document: &Json, properties: &Json) -> Result<(), VerifyError> {
    let references = match signature
        .first("SignedInfo")
        .and_then(|info| info.get("Reference"))
    {
        Some(Json::Array(references)) => references,
        _ => return Err(VerifyError::Unsigned),
    };
    let digest_of = |uri: &str| {
        references
            .iter()
            .find(
                |reference| matches!(reference.get("URI"), Some(Json::String(text)) if text == uri),
            )
            .and_then(|reference| reference.text_at(&["DigestValue"]))
    };
    if digest_of("") != Some(document_digest(document).as_str()) {
        return Err(VerifyError::DocumentDigestMismatch);
    }
    if digest_of("#id-xades-signed-props") != Some(digest(properties.encode().as_bytes()).as_str())
    {
        return Err(VerifyError::SignedPropertiesDigestMismatch);
    }
    Ok(())
}

fn check_certificate(properties: &Json, certificate: &Certificate) -> Result<(), VerifyError> {
    let cert = properties
        .at(&[
            "SignedProperties",
            "SignedSignatureProperties",
            "SigningCertificate",
            "Cert",
        ])
        .ok_or(VerifyError::Unsigned)?;
    if cert.text_at(&["CertDigest", "DigestValue"]) != Some(digest(certificate.der()).as_str()) {
        return Err(VerifyError::CertificateDigestMismatch);
    }
    if cert.text_at(&["IssuerSerial", "X509IssuerName"]) != Some(certificate.issuer_name.as_str())
        || cert.text_at(&["IssuerSerial", "X509SerialNumber"])
            != Some(certificate.serial_number.as_str())
    {
        return Err(VerifyError::IssuerSerialMismatch);
    }
    Ok(())
}

fn decode64(value: Option<&str>) -> Result<Vec<u8>, VerifyError> {
    let value = value.ok_or(VerifyError::Unsigned)?;
    BASE64.decode(value).map_err(|_| VerifyError::InvalidBase64)
}
