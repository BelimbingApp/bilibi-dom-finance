//! The MyInvois e-invoicing API, through Base Integration (Bilimbi
//! `Bilimbi.Finance.Einvoice.MyInvois.Client` and its `TokenCache`).
//!
//! Every call goes out through `Integration::send` as the person it acts for
//! (`Origin::User`), so each exchange is recorded under them, without bodies
//! or headers. A call takes the company's [`Credentials`] for one environment
//! and the API's base URL (`finance.einvoice.<environment>.api_url`).
//!
//! Logging in (`POST /connect/token`, client credentials, scope
//! `InvoicingAPI`) yields a token that lives an hour; the token cache keeps it
//! per company, environment and client ID until two minutes before it lapses,
//! and a 401 drops it. The authority allows 12 logins a minute, so a token is
//! reused rather than fetched per call. A token is a short-lived credential:
//! it lives only in this process's memory, never in the database, a log or an
//! exchange record.
//!
//! Results keep the authority's answer intact so a caller can store it as the
//! record: a 2xx answer is [`Reply`]; anything else is a [`CallError`], where
//! a refusal the authority explains carries its status and body, a 422 or 429
//! carries the `Retry-After` seconds when the authority sent them, and a
//! transport failure names its class. Nothing here retries: resubmitting after
//! a 2xx is never safe, so retries belong to the caller that recorded the
//! submission first.
use crate::authority::{Connection, Credentials};
use bilibi_integration::{Call, Failure, Origin, Outgoing};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// The system every exchange is recorded under.
const SYSTEM: &str = "myinvois";
/// The owner type of every exchange: the module, for the company it serves.
const OWNER_TYPE: &str = "finance/einvoice";
/// Renew this long before the authority's expiry, so a request never leaves
/// with a token that lapses in flight.
const MARGIN_SECONDS: i64 = 120;

/// A 2xx answer: the HTTP status and the decoded body (`null` when it was not
/// JSON).
#[derive(Clone, Debug)]
pub(crate) struct Reply {
    pub status: u16,
    pub body: Value,
}

/// Why a call has no 2xx answer.
#[derive(Clone, Debug)]
pub(crate) enum CallError {
    /// The company has no client ID or secret for the environment.
    MissingCredentials,
    /// An identifier that is not one the authority issues; it never reaches a
    /// URL path.
    InvalidIdentifier,
    /// A cancellation reason that is blank or longer than 300 characters.
    InvalidReason,
    /// The authority answered with this status and body; `retry_after` is the
    /// seconds it asked for on a 422 or 429.
    Http {
        status: u16,
        body: Value,
        retry_after: Option<u64>,
    },
    /// The login answered 2xx without a usable token.
    UnexpectedTokenResponse(u16),
    /// No answer: Integration's failure class (`timeout`, `connect`, ...), or
    /// `invalid`/`unrecorded` when the call was not made or not recorded.
    Transport(&'static str),
}

pub(crate) type Result = std::result::Result<Reply, CallError>;

type TokenKey = (i64, String, String);

/// Live tokens by company, environment and client ID, with the Unix second
/// each expires at.
static TOKENS: LazyLock<Mutex<HashMap<TokenKey, (String, i64)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn cache_key(connection: &Connection<'_>) -> TokenKey {
    let credentials = connection.credentials;
    (
        connection.company_id,
        credentials.environment.clone(),
        credentials.client_id.clone().unwrap_or_default(),
    )
}

fn cached_token(key: &TokenKey) -> Option<String> {
    let tokens = TOKENS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tokens
        .get(key)
        .filter(|(_, expires_at)| expires_at - MARGIN_SECONDS > unix_now())
        .map(|(token, _)| token.clone())
}

fn keep_token(key: TokenKey, token: String, expires_in: i64) {
    let mut tokens = TOKENS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tokens.insert(key, (token, unix_now() + expires_in));
}

fn drop_token(key: &TokenKey) {
    let mut tokens = TOKENS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    tokens.remove(key);
}

/// Whether the credentials name a client ID and secret.
fn complete(credentials: &Credentials) -> bool {
    credentials
        .client_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        && credentials
            .client_secret
            .as_deref()
            .is_some_and(|secret| !secret.is_empty())
}

/// An identifier the authority issued: 1 to 64 letters, digits or dashes.
fn path_segment(value: &str) -> std::result::Result<&str, CallError> {
    if (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        Ok(value)
    } else {
        Err(CallError::InvalidIdentifier)
    }
}

/// `application/x-www-form-urlencoded` text of one value.
fn form_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char)
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

fn query(pairs: &[(&str, String)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{key}={}", form_encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Logs in afresh, bypassing and then refreshing the token cache: the "Test
/// connection" check.
pub(crate) async fn login(connection: &Connection<'_>) -> Result {
    if !complete(connection.credentials) {
        return Err(CallError::MissingCredentials);
    }
    drop_token(&cache_key(connection));
    fetch_token(connection).await
}

/// Submits documents: each the format, the exact bytes and the company's code
/// number. A document is sent base64-encoded with the hex SHA-256 of its
/// bytes; the authority answers 202 with a `submissionUid` and its accepted
/// and rejected documents. The envelope is written by the module's ordered
/// JSON writer, like every document the authority reads.
pub(crate) async fn submit(connection: &Connection<'_>, body: Vec<u8>) -> Result {
    authorized(
        connection,
        "documents.submit",
        Outgoing::post(url(connection, "/api/v1.0/documentsubmissions/"))
            .header("Content-Type", "application/json")
            .bytes(body),
    )
    .await
}

/// A submission's overall status and its documents' summaries, one page.
pub(crate) async fn submission_status(connection: &Connection<'_>, submission_uid: &str) -> Result {
    let uid = path_segment(submission_uid)?;
    let params = query(&[("pageNo", "1".into()), ("pageSize", "100".into())]);
    authorized(
        connection,
        "submissions.status",
        Outgoing::get(url(
            connection,
            &format!("/api/v1.0/documentsubmissions/{uid}?{params}"),
        )),
    )
    .await
}

/// A document's details, with its validation results and `longId`.
pub(crate) async fn document_details(connection: &Connection<'_>, uuid: &str) -> Result {
    let uuid = path_segment(uuid)?;
    authorized(
        connection,
        "documents.details",
        Outgoing::get(url(
            connection,
            &format!("/api/v1.0/documents/{uuid}/details"),
        )),
    )
    .await
}

/// The documents the taxpayer sent between `from` and `to` (at most 31 days
/// apart), one page. Each names its `uuid`, `submissionUid` and `internalId`,
/// the code number the submitter gave it.
pub(crate) async fn recent_documents(
    connection: &Connection<'_>,
    from: jiff::Timestamp,
    to: jiff::Timestamp,
    page: u32,
) -> Result {
    let instant = |at: jiff::Timestamp| at.strftime("%Y-%m-%dT%H:%M:%SZ").to_string();
    let params = query(&[
        ("pageNo", page.to_string()),
        ("pageSize", "100".into()),
        ("submissionDateFrom", instant(from)),
        ("submissionDateTo", instant(to)),
        ("InvoiceDirection", "Sent".into()),
    ]);
    authorized(
        connection,
        "documents.recent",
        Outgoing::get(url(
            connection,
            &format!("/api/v1.0/documents/recent?{params}"),
        )),
    )
    .await
}

/// Cancels an issued document with a reason of at most 300 characters.
pub(crate) async fn cancel_document(
    connection: &Connection<'_>,
    uuid: &str,
    reason: &str,
) -> Result {
    let uuid = path_segment(uuid)?;
    if reason.trim().is_empty() || reason.chars().count() > 300 {
        return Err(CallError::InvalidReason);
    }
    let body = super::json::Json::Object(vec![
        ("status".into(), super::json::Json::str("cancelled")),
        ("reason".into(), super::json::Json::str(reason)),
    ]);
    authorized(
        connection,
        "documents.cancel",
        Outgoing::new(
            http::Method::PUT,
            url(
                connection,
                &format!("/api/v1.0/documents/state/{uuid}/state"),
            ),
        )
        .header("Content-Type", "application/json")
        .bytes(body.encode().into_bytes()),
    )
    .await
}

fn url(connection: &Connection<'_>, path: &str) -> String {
    format!("{}{path}", connection.api_url.trim_end_matches('/'))
}

/// A call with the cached token, or a fresh one; a 401 drops the token so the
/// next call logs in again.
async fn authorized(connection: &Connection<'_>, operation: &str, request: Outgoing) -> Result {
    if !complete(connection.credentials) {
        return Err(CallError::MissingCredentials);
    }
    let key = cache_key(connection);
    let token = match cached_token(&key) {
        Some(token) => token,
        None => {
            let reply = fetch_token(connection).await?;
            match reply.body.get("access_token").and_then(Value::as_str) {
                Some(token) => token.to_owned(),
                None => return Err(CallError::UnexpectedTokenResponse(reply.status)),
            }
        }
    };
    let result = send(
        connection,
        operation,
        request.header("Authorization", format!("Bearer {token}")),
    )
    .await;
    if let Err(CallError::Http { status: 401, .. }) = &result {
        drop_token(&key);
    }
    result
}

async fn fetch_token(connection: &Connection<'_>) -> Result {
    let credentials = connection.credentials;
    let form = query(&[
        (
            "client_id",
            credentials.client_id.clone().unwrap_or_default(),
        ),
        (
            "client_secret",
            credentials.client_secret.clone().unwrap_or_default(),
        ),
        ("grant_type", "client_credentials".into()),
        ("scope", "InvoicingAPI".into()),
    ]);
    let reply = send(
        connection,
        "token",
        Outgoing::post(url(connection, "/connect/token"))
            .header("Content-Type", "application/x-www-form-urlencoded")
            .bytes(form.into_bytes()),
    )
    .await?;
    let token = reply.body.get("access_token").and_then(Value::as_str);
    let expires_in = reply.body.get("expires_in").and_then(Value::as_i64);
    match (token, expires_in) {
        (Some(token), Some(expires_in)) if !token.is_empty() && expires_in > 0 => {
            keep_token(cache_key(connection), token.to_owned(), expires_in);
            // The token goes to the caller that asked for it, never further.
            Ok(reply)
        }
        _ => Err(CallError::UnexpectedTokenResponse(reply.status)),
    }
}

async fn send(connection: &Connection<'_>, operation: &str, request: Outgoing) -> Result {
    let call = Call {
        system: SYSTEM.into(),
        operation: operation.into(),
        owner_type: Some(OWNER_TYPE.into()),
        owner_id: Some(connection.company_id),
        ..Call::default()
    };
    match connection
        .integration
        .send(Origin::User(connection.scope), &call, request)
        .await
    {
        Ok(reply) => {
            let body = serde_json::from_slice(&reply.body).unwrap_or(Value::Null);
            if (200..300).contains(&reply.status) {
                Ok(Reply {
                    status: reply.status,
                    body,
                })
            } else {
                let retry_after = matches!(reply.status, 422 | 429)
                    .then(|| reply.headers.get("retry-after"))
                    .flatten()
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.trim().parse::<u64>().ok());
                Err(CallError::Http {
                    status: reply.status,
                    body,
                    retry_after,
                })
            }
        }
        Err(Failure::Transport { class, .. }) => Err(CallError::Transport(class)),
        Err(Failure::Invalid) => Err(CallError::Transport("invalid")),
        // The authority may have acted; the caller's own record decides what
        // happens next, never a blind repeat.
        Err(Failure::Unrecorded) => Err(CallError::Transport("unrecorded")),
    }
}
