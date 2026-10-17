//! A company's signing certificate and private key, read from PEM text
//! (Bilimbi `Bilimbi.Finance.Einvoice.Certificate`, its validators).
//!
//! A company exports its certificate and its unencrypted private key (`BEGIN
//! PRIVATE KEY` or `BEGIN RSA PRIVATE KEY`) as PEM and stores both as
//! encrypted settings. PKCS#12 is not read. Only RSA keys are accepted: the
//! authority's signature is RSA PKCS#1 v1.5 with SHA-256.
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::RsaPrivateKey;
use x509_parser::pem::Pem;

/// Settings validator: true when `pem` holds one readable X.509 certificate
/// (the first PEM entry is a certificate; entries after it are ignored).
pub(crate) fn valid_certificate_pem(pem: &str) -> bool {
    match Pem::iter_from_buffer(pem.as_bytes()).next() {
        Some(Ok(entry)) => entry.label == "CERTIFICATE" && entry.parse_x509().is_ok(),
        _ => false,
    }
}

/// Settings validator: true when `pem` holds exactly one unencrypted RSA
/// private key.
pub(crate) fn valid_private_key_pem(pem: &str) -> bool {
    let entries: Result<Vec<Pem>, _> = Pem::iter_from_buffer(pem.as_bytes()).collect();
    let Ok(entries) = entries else {
        return false;
    };
    match entries.as_slice() {
        [entry] => match entry.label.as_str() {
            "PRIVATE KEY" => RsaPrivateKey::from_pkcs8_der(&entry.contents).is_ok(),
            "RSA PRIVATE KEY" => RsaPrivateKey::from_pkcs1_der(&entry.contents).is_ok(),
            _ => false,
        },
        _ => false,
    }
}
