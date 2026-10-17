//! The submit and poll jobs (Bilimbi `Bilimbi.Finance.Einvoice.SubmitWorker`
//! and `PollWorker`), Base Queue workers queued with `Queue::enqueue_for` by
//! the facade, so each runs as the person who asked; the queue re-proves that
//! person and their `finance.einvoice.documents.submit` grant on every
//! attempt, and a job without one is cancelled (`actor_refused`). The
//! arguments are the company and document IDs and nothing else.
//!
//! An accepted or refused submission completes the submit job; a rate limit
//! or an outage retries it under the queue's backoff (`min(3600, 2^attempt)`
//! seconds), and its hundred attempts span about four days, which covers the
//! authority's 72-hour outage allowance after which the `overdue` notice is
//! recorded; a duplicate stops it for an operator. The poll job reads a
//! submission's progress a bounded number of times per attempt and retries
//! while the authority is still processing.
use crate::clearance::{Writer, POLL_WORKER, SUBMIT_WORKER};
use crate::contributions::provenance;
use crate::{Einvoice, MODULE, SUBMIT};
use bilibi_database::{Error, Result};
use bilibi_module_registry::{Contribution, Provenance};
use bilibi_queue::execution::{
    Execution, Handler, HandlerBinding, HandlerFuture, HandlerService, Identity, Outcome, HANDLERS,
};
use bilibi_queue::{Worker, WORKERS};
use jiff::Timestamp;
use serde_json::{json, Value};
use std::sync::Arc;

/// The registry service of the submit job's handler.
pub(crate) const SUBMIT_SERVICE: &str = "finance/einvoice.submit_job";
/// The registry service of the poll job's handler.
pub(crate) const POLL_SERVICE: &str = "finance/einvoice.poll_job";

/// An exact allowlist: the two identifiers and nothing else reach the queue.
fn validate_args(args: Value) -> Result<Value> {
    let object = args.as_object().ok_or(Error::Invalid)?;
    let id = |key: &str| object.get(key).and_then(Value::as_i64).filter(|id| *id > 0);
    match (object.len(), id("company_id"), id("document_id")) {
        (2, Some(company_id), Some(document_id)) => {
            Ok(json!({ "company_id": company_id, "document_id": document_id }))
        }
        _ => Err(Error::Invalid),
    }
}

static SUBMIT_JOB: Worker = Worker {
    max_attempts: 100,
    validate: validate_args,
};
static POLL_JOB: Worker = Worker {
    max_attempts: 20,
    validate: validate_args,
};
inventory::submit! { Contribution::new(WORKERS, SUBMIT_WORKER, provenance!(), &SUBMIT_JOB) }
inventory::submit! { Contribution::new(WORKERS, POLL_WORKER, provenance!(), &POLL_JOB) }
inventory::submit! { Contribution::new(HANDLERS, SUBMIT_WORKER, provenance!(), &HandlerBinding {
    service: SUBMIT_SERVICE,
    capability: SUBMIT,
}) }
inventory::submit! { Contribution::new(HANDLERS, POLL_WORKER, provenance!(), &HandlerBinding {
    service: POLL_SERVICE,
    capability: SUBMIT,
}) }

/// Which of the two jobs a handler runs.
#[derive(Clone, Copy)]
pub(crate) enum Job {
    Submit,
    Poll,
}

/// The queue handler of a job, bound to the facade.
pub(crate) struct ClearanceJob {
    pub(crate) einvoice: Arc<Einvoice>,
    pub(crate) job: Job,
}

impl Handler for ClearanceJob {
    fn perform<'a>(&'a self, execution: &'a Execution, args: Value) -> HandlerFuture<'a> {
        Box::pin(async move {
            let Identity::User(scope) = execution.identity() else {
                return Outcome::Cancel("actor_unavailable");
            };
            let Ok(args) = validate_args(args) else {
                return Outcome::Cancel("invalid_document");
            };
            let (company_id, document_id) = (
                args["company_id"].as_i64().unwrap_or_default(),
                args["document_id"].as_i64().unwrap_or_default(),
            );
            let writer = Writer::Job(execution);
            match self.job {
                Job::Submit => {
                    self.einvoice
                        .submission(scope, company_id, document_id, Timestamp::now(), writer)
                        .await
                }
                Job::Poll => {
                    self.einvoice
                        .poll(scope, company_id, document_id, writer)
                        .await
                }
            }
        })
    }
}

/// A job's handler service, beside the facade in the module's descriptor.
pub(crate) fn handler(
    services: &bilibi_module_registry::Services,
    job: Job,
) -> bilibi_module_registry::Result<bilibi_module_registry::Value> {
    Ok(Arc::new(HandlerService(Arc::new(ClearanceJob {
        einvoice: services.get(crate::SERVICE)?,
        job,
    }))))
}
