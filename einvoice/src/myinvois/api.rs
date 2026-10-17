//! The MyInvois API's answers in meta-terms (Bilimbi `MyInvois`'s `submit/4`,
//! `submission_progress/4`, `find_submission/4`, `document_status/4`,
//! `cancel/5` and `login/3`): each call through [`super::client`] becomes an
//! outcome the clearance records, with the authority's answer kept intact.
use super::client::{self, CallError, Reply};
use super::json::Json;
use crate::authority::{
    Answer, AuthorityStatus, CancelOutcome, CancelRefusal, Connection, DocumentStatus,
    LoginOutcome, Lookup, Payload, Progress, Search, StatusRead, SubmitOutcome,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use jiff::{SignedDuration, Timestamp};
use rsa::sha2::{Digest, Sha256};
use serde_json::Value;

/// The authority lets one search span at most this many days.
const SEARCH_DAYS: i64 = 30;
/// A search reads at most this many pages before it gives up.
const SEARCH_PAGES: u32 = 10;

/// The lowercase hex SHA-256 of `bytes`, as the authority and the
/// submissions table both write it.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn answer(status: u16, body: Value) -> Answer {
    Answer {
        http_status: Some(status),
        response: Some(body),
        ..Answer::default()
    }
}

/// An answer that is not a 2xx, in the outcome it implies: a rate limit (a
/// 429, or a 422 with `Retry-After`), an outage (a 5xx or no answer), or a
/// refusal the authority explained.
enum Failure {
    RateLimited(Answer),
    Unavailable(Answer),
    Refused(Answer),
}

fn failure(error: CallError) -> Failure {
    match error {
        CallError::Http {
            status,
            body,
            retry_after: Some(seconds),
        } => Failure::RateLimited(Answer {
            retry_after: Some(seconds),
            ..answer(status, body)
        }),
        CallError::Http {
            status: 429, body, ..
        } => Failure::RateLimited(answer(429, body)),
        CallError::Http { status, body, .. } if status >= 500 => Failure::Unavailable(Answer {
            reason: Some(format!("HTTP {status}")),
            ..answer(status, body)
        }),
        CallError::Http { status, body, .. } => Failure::Refused(Answer {
            reason: Some(error_message(&body).unwrap_or_else(|| format!("HTTP {status}"))),
            ..answer(status, body)
        }),
        CallError::Transport(class) => Failure::Unavailable(Answer {
            reason: Some(class.into()),
            ..Answer::default()
        }),
        CallError::UnexpectedTokenResponse(status) => Failure::Refused(Answer {
            http_status: Some(status),
            reason: Some("unexpected login answer".into()),
            ..Answer::default()
        }),
        CallError::MissingCredentials => Failure::Refused(Answer {
            reason: Some("missing credentials".into()),
            ..Answer::default()
        }),
        CallError::InvalidIdentifier => Failure::Refused(Answer {
            reason: Some("invalid identifier".into()),
            ..Answer::default()
        }),
        CallError::InvalidReason => Failure::Refused(Answer {
            reason: Some("invalid reason".into()),
            ..Answer::default()
        }),
    }
}

/// The authority's error code (`{"error": {"code": ...}}`).
fn error_code(body: &Value) -> Option<&str> {
    body.get("error")?.get("code")?.as_str()
}

/// The authority's explanation: the error's message and its details'
/// messages, joined, in the first 300 characters; or its code.
fn error_message(body: &Value) -> Option<String> {
    let error = body.get("error").unwrap_or(body);
    if let Some(message) = error.get("message").and_then(Value::as_str) {
        let mut parts = vec![message.to_owned()];
        if let Some(details) = error.get("details").and_then(Value::as_array) {
            parts.extend(
                details
                    .iter()
                    .filter_map(|detail| detail.get("message").and_then(Value::as_str))
                    .map(str::to_owned),
            );
        }
        return Some(parts.join("; ").chars().take(300).collect());
    }
    error.get("code").and_then(Value::as_str).map(str::to_owned)
}

fn text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn instant(value: Option<&Value>) -> Option<Timestamp> {
    value.and_then(Value::as_str)?.parse().ok()
}

pub(crate) async fn login(connection: &Connection<'_>) -> LoginOutcome {
    match client::login(connection).await {
        Ok(_) => LoginOutcome::Succeeded,
        Err(CallError::MissingCredentials) => LoginOutcome::MissingCredentials,
        Err(CallError::Http { status, retry_after: Some(seconds), .. }) => LoginOutcome::RateLimited { status, seconds },
        Err(CallError::Http { status, .. }) => LoginOutcome::Refused(status),
        Err(CallError::Transport(_)) => LoginOutcome::Unreachable,
        Err(_) => LoginOutcome::Failed,
    }
}

pub(crate) async fn submit(
    connection: &Connection<'_>,
    payload: &Payload,
    code_number: &str,
) -> SubmitOutcome {
    let body = Json::object([(
        "documents",
        Some(Json::Array(vec![Json::object([
            ("format", Some(Json::str(payload.format))),
            ("document", Some(Json::str(&BASE64.encode(&payload.bytes)))),
            ("documentHash", Some(Json::str(&sha256_hex(&payload.bytes)))),
            ("codeNumber", Some(Json::str(code_number))),
        ])])),
    )]);
    match client::submit(connection, body.encode().into_bytes()).await {
        Ok(Reply { status, body }) => submission_outcome(code_number, status, body),
        Err(CallError::Http {
            status: 422,
            body,
            retry_after: Some(seconds),
        }) => SubmitOutcome::Duplicate(Answer {
            retry_after: Some(seconds),
            ..answer(422, body)
        }),
        Err(CallError::Http {
            status: 422, body, ..
        }) => {
            if error_code(&body) == Some("DuplicateSubmission") {
                SubmitOutcome::Duplicate(answer(422, body))
            } else {
                SubmitOutcome::Refused(Answer {
                    reason: error_message(&body),
                    ..answer(422, body)
                })
            }
        }
        Err(error) => match failure(error) {
            Failure::RateLimited(answer) => SubmitOutcome::RateLimited(answer),
            Failure::Unavailable(answer) => SubmitOutcome::Unavailable(answer),
            Failure::Refused(answer) => SubmitOutcome::Refused(answer),
        },
    }
}

/// A 202 lists each document as accepted (with its UUID) or rejected (with
/// the authority's error); a document missing from both lists is treated as
/// refused, never as accepted.
fn submission_outcome(code_number: &str, status: u16, body: Value) -> SubmitOutcome {
    let Some(uid) = text(body.get("submissionUid")) else {
        return SubmitOutcome::Refused(Answer {
            reason: Some("no submission UID in the answer".into()),
            ..answer(status, body)
        });
    };
    let listed = |key: &str| {
        body.get(key)
            .and_then(Value::as_array)
            .and_then(|entries| {
                entries.iter().find(|entry| {
                    entry.get("invoiceCodeNumber").and_then(Value::as_str) == Some(code_number)
                })
            })
            .cloned()
    };
    let accepted = listed("acceptedDocuments");
    let rejected = listed("rejectedDocuments");
    let base = Answer {
        submission_uid: Some(uid),
        ..answer(status, body)
    };
    match (accepted.and_then(|entry| text(entry.get("uuid"))), rejected) {
        (Some(uuid), _) => SubmitOutcome::Accepted(Answer {
            document_uuid: Some(uuid),
            ..base
        }),
        (None, Some(rejected)) => SubmitOutcome::Refused(Answer {
            reason: rejected
                .get("error")
                .and_then(|error| error_message(&serde_json::json!({ "error": error.clone() }))),
            ..base
        }),
        (None, None) => SubmitOutcome::Refused(Answer {
            reason: Some("not listed as accepted".into()),
            ..base
        }),
    }
}

pub(crate) async fn submission_progress(
    connection: &Connection<'_>,
    submission_uid: &str,
) -> Progress {
    match client::submission_status(connection, submission_uid).await {
        Ok(Reply { status, body }) => match body.get("overallStatus") {
            Some(overall) => {
                let overall = overall
                    .as_str()
                    .map_or_else(|| overall.to_string(), str::to_owned);
                if overall.to_lowercase() == "in progress" {
                    Progress::InProgress(answer(status, body))
                } else {
                    Progress::Done(answer(status, body))
                }
            }
            None => Progress::Refused(Answer {
                reason: Some("no overall status in the answer".into()),
                ..answer(status, body)
            }),
        },
        Err(error) => match failure(error) {
            Failure::RateLimited(answer) => Progress::RateLimited(answer),
            Failure::Unavailable(answer) => Progress::Unavailable(answer),
            Failure::Refused(answer) => Progress::Refused(answer),
        },
    }
}

pub(crate) async fn find_submission(connection: &Connection<'_>, lookup: &Lookup<'_>) -> Search {
    let earliest = lookup.until - SignedDuration::from_hours(SEARCH_DAYS * 24);
    let from = lookup.since.max(earliest);
    let mut page = 1;
    loop {
        let (status, body) =
            match client::recent_documents(connection, from, lookup.until, page).await {
                Ok(Reply { status, body }) => (status, body),
                Err(error) => {
                    return match failure(error) {
                        Failure::RateLimited(answer) => Search::RateLimited(answer),
                        Failure::Unavailable(answer) => Search::Unavailable(answer),
                        Failure::Refused(answer) => Search::Refused(answer),
                    }
                }
            };
        let Some(results) = body.get("result").and_then(Value::as_array) else {
            return Search::Refused(Answer {
                reason: Some("no document list in the answer".into()),
                ..answer(status, body)
            });
        };
        let found = results
            .iter()
            .filter(|entry| {
                entry.get("internalId").and_then(Value::as_str) == Some(lookup.code_number)
                    && !text(entry.get("uuid")).is_some_and(|uuid| lookup.exclude.contains(&uuid))
            })
            .max_by_key(|entry| {
                entry
                    .get("dateTimeReceived")
                    .map(|received| {
                        received
                            .as_str()
                            .map_or_else(|| received.to_string(), str::to_owned)
                    })
                    .unwrap_or_default()
            });
        if let Some(entry) = found {
            if let (Some(uuid), Some(uid)) =
                (text(entry.get("uuid")), text(entry.get("submissionUid")))
            {
                return Search::Found(Answer {
                    submission_uid: Some(uid),
                    document_uuid: Some(uuid),
                    reason: Some("found at the authority by its code number".into()),
                    ..answer(status, entry.clone())
                });
            }
        }
        let total = body
            .get("metadata")
            .and_then(|metadata| metadata.get("totalPages"))
            .and_then(Value::as_u64);
        if !total.is_some_and(|total| u64::from(page) < total) {
            return Search::NotFound(answer(status, body));
        }
        if page >= SEARCH_PAGES {
            return Search::Refused(Answer {
                reason: Some("search window too large to read".into()),
                ..answer(status, body)
            });
        }
        page += 1;
    }
}

pub(crate) async fn document_status(
    connection: &Connection<'_>,
    document_uuid: &str,
) -> StatusRead {
    match client::document_details(connection, document_uuid).await {
        Ok(Reply { status, body }) => {
            let Some(name) = body.get("status") else {
                return StatusRead::Refused(Answer {
                    reason: Some("no status in the answer".into()),
                    ..answer(status, body)
                });
            };
            let known = match name.as_str().map(str::to_lowercase).as_deref() {
                Some("submitted") => Some(AuthorityStatus::Submitted),
                Some("valid") => Some(AuthorityStatus::Valid),
                Some("invalid") => Some(AuthorityStatus::Invalid),
                Some("cancelled") => Some(AuthorityStatus::Cancelled),
                _ => None,
            };
            match known {
                None => StatusRead::Refused(Answer {
                    reason: Some(format!("unknown status {name}")),
                    ..answer(status, body)
                }),
                Some(known) => StatusRead::Read(DocumentStatus {
                    status: known,
                    long_id: text(body.get("longId")),
                    validated_at: instant(body.get("dateTimeValidated")),
                    cancelled_at: instant(body.get("cancelDateTime")),
                    rejection_requested_at: instant(body.get("rejectRequestDateTime")),
                    answer: Answer {
                        reason: text(body.get("documentStatusReason")),
                        ..answer(status, body)
                    },
                }),
            }
        }
        Err(error) => match failure(error) {
            Failure::RateLimited(answer) => StatusRead::RateLimited(answer),
            Failure::Unavailable(answer) => StatusRead::Unavailable(answer),
            Failure::Refused(answer) => StatusRead::Refused(answer),
        },
    }
}

pub(crate) async fn cancel(
    connection: &Connection<'_>,
    document_uuid: &str,
    reason: &str,
) -> CancelOutcome {
    match client::cancel_document(connection, document_uuid, reason).await {
        Ok(Reply { status, body }) => CancelOutcome::Cancelled(answer(status, body)),
        Err(CallError::Http { status, body, .. })
            if (400..500).contains(&status) && status != 429 =>
        {
            let code = match error_code(&body) {
                Some("OperationPeriodOver") => CancelRefusal::PeriodOver,
                Some("IncorrectState") => CancelRefusal::IncorrectState,
                Some("ActiveReferencingDocuments") => CancelRefusal::ActiveReferencingDocuments,
                _ => CancelRefusal::Other,
            };
            CancelOutcome::Refused(
                code,
                Answer {
                    reason: error_message(&body),
                    ..answer(status, body)
                },
            )
        }
        Err(CallError::InvalidReason) => CancelOutcome::Refused(
            CancelRefusal::Other,
            Answer {
                reason: Some("invalid reason".into()),
                ..Answer::default()
            },
        ),
        Err(error) => match failure(error) {
            Failure::RateLimited(answer) => CancelOutcome::RateLimited(answer),
            Failure::Unavailable(answer) => CancelOutcome::Unavailable(answer),
            Failure::Refused(answer) => CancelOutcome::Refused(CancelRefusal::Other, answer),
        },
    }
}
