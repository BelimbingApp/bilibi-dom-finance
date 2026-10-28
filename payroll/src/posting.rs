//! Atomic recognition/reversal and attempt history, mirroring Bilimbi
//! Payroll.Posting. Source facts are read only from the owned event projection.
use crate::{
    authorize, ledger_error, reason, source, store, Mapping, Payroll, PayrollError, Posting,
    Result, Reversal, Settings, MODULE, POST_POSTINGS,
};
use bilibi_database::{Error, MutationTransaction};
use bilibi_finance_ledger::{Book, EntryOwner, LedgerError, Money, NewEntry, NewLine, PostOptions};
use bilibi_settings::Target;
use bilibi_tenancy::Scope;
use jiff::{civil::Date, tz::TimeZone, Timestamp};
use rust_decimal::{Decimal, RoundingStrategy};
use std::{collections::BTreeMap, error::Error as StdError};

enum Document {
    Run(source::Run),
    Claim(source::Claim),
}
impl Document {
    fn source(&self) -> store::Source<'_> {
        match self {
            Self::Run(r) => store::Source {
                kind: "payroll_run",
                id: r.id,
                reference: &r.reference,
            },
            Self::Claim(c) => store::Source {
                kind: "claim",
                id: c.id,
                reference: &c.reference,
            },
        }
    }
    fn currency(&self) -> &str {
        match self {
            Self::Run(r) => &r.currency_code,
            Self::Claim(c) => &c.currency_code,
        }
    }
}

impl Payroll {
    pub async fn post_payroll_run(&self, scope: &Scope, company: i64, id: i64) -> Result<Posting> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company, POST_POSTINGS).await?;
        self.require_source()?;
        let run = source::run(&**client, scope, company, id).await?;
        if !run.finalised {
            return Err(PayrollError::NotFinalised);
        }
        drop(client);
        self.post(scope, company, &Document::Run(run)).await
    }

    pub async fn post_claim(&self, scope: &Scope, company: i64, id: i64) -> Result<Posting> {
        self.post_claims(scope, company, &[id])
            .await?
            .pop()
            .ok_or(PayrollError::SourceNotFound)?
            .1
    }

    /// One source snapshot, ordered individual outcomes, separate transactions.
    /// A whole-source refusal (including unavailable) makes no attempt events.
    pub async fn post_claims(
        &self,
        scope: &Scope,
        company: i64,
        ids: &[i64],
    ) -> Result<Vec<(i64, Result<Posting>)>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company, POST_POSTINGS).await?;
        self.require_source()?;
        let claims = source::claims(&**client, scope, company).await?;
        drop(client);
        let mut results = Vec::new();
        for id in ids {
            let outcome = match claims.iter().find(|c| c.id == *id) {
                Some(c) => {
                    self.post(
                        scope,
                        company,
                        &Document::Claim(source::Claim {
                            id: c.id,
                            reference: c.reference.clone(),
                            currency_code: c.currency_code.clone(),
                            approved_on: c.approved_on,
                            amount: c.amount,
                            category_code: c.category_code.clone(),
                        }),
                    )
                    .await
                }
                None => Err(PayrollError::SourceNotFound),
            };
            results.push((*id, outcome));
        }
        Ok(results)
    }

    async fn post(&self, scope: &Scope, company: i64, document: &Document) -> Result<Posting> {
        let mut outcome = self.post_once(scope, company, document).await;
        if outcome.as_ref().is_err_and(transient) {
            // A lost COMMIT acknowledgement is not evidence of rollback.
            // Reconcile durable success before trying the transaction again.
            if let Some(post) = self
                .committed(scope, company, document.source(), false)
                .await?
            {
                return Ok(post);
            }
            outcome = self.post_once(scope, company, document).await;
        }
        match outcome {
            Ok(posting) => Ok(posting),
            Err(PayrollError::AlreadyPosted) => Err(PayrollError::AlreadyPosted),
            Err(error) => {
                self.failed(scope, company, document.source(), false, error)
                    .await
            }
        }
    }

    async fn post_once(&self, scope: &Scope, company: i64, document: &Document) -> Result<Posting> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .ledger
            .write(
                &mut client,
                scope,
                company,
                &[POST_POSTINGS],
                "finance.payroll.post",
            )
            .await
            .map_err(ledger_error)?;
        let identity = document.source();
        if store::already_posted(
            tx.sql(),
            scope.tenant(),
            company,
            identity.kind,
            identity.id,
        )
        .await?
        {
            return Err(PayrollError::AlreadyPosted);
        }
        let refreshed;
        let document = match document {
            Document::Run(run) => {
                refreshed = Document::Run(source::run(tx.sql(), scope, company, run.id).await?);
                &refreshed
            }
            other => other,
        };
        let settings: Settings = store::decode(
            store::settings(tx.sql(), scope.tenant(), company)
                .await?
                .ok_or_else(|| failed("payroll posting accounts not set"))?,
        )?;
        let mappings: Vec<Mapping> = store::mappings(tx.sql(), scope.tenant(), company)
            .await?
            .into_iter()
            .map(store::decode)
            .collect::<Result<_>>()?;
        let book = self.ledger.get_book_on(tx.sql(), scope, company).await?;
        if book.currency_code != document.currency() {
            return Err(failed(format!(
                "the book is kept in {}, the source in {}",
                book.currency_code,
                document.currency()
            )));
        }
        let new = build(document, &book, &settings, &mappings)?;
        let total = total(&new.lines, book.minor_units)?;
        let identity = document.source();
        let owner = owner(&identity);
        let entry = self
            .ledger
            .post_entry_in(
                &mut tx,
                scope,
                company,
                &new,
                &PostOptions {
                    owner: Some(owner),
                    ..PostOptions::default()
                },
            )
            .await?;
        let id = store::record(
            &mut tx,
            scope,
            company,
            identity,
            store::Outcome::Posted(&entry, total),
        )
        .await?;
        let posting = store::posting(tx.sql(), scope.tenant(), company, id).await?;
        tx.commit().await?;
        Ok(posting)
    }

    /// Source-independent correction of an existing local posting. A reversed
    /// source remains posted forever; it cannot be recognised a second time.
    pub async fn reverse_posting(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        new: &Reversal,
    ) -> Result<Posting> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company, POST_POSTINGS).await?;
        let why = reason(Some(&new.reason), true)?.expect("required reason");
        let original = store::posting(&**client, scope.tenant(), company, id).await?;
        drop(client);
        let mut outcome = self
            .reverse_once(scope, company, id, new.entry_date, &why)
            .await;
        if outcome.as_ref().is_err_and(transient) {
            if let Some(post) = self
                .committed(scope, company, identity(&original), true)
                .await?
            {
                return Ok(post);
            }
            outcome = self
                .reverse_once(scope, company, id, new.entry_date, &why)
                .await;
        }
        match outcome {
            Ok(posting) => Ok(posting),
            Err(PayrollError::AlreadyReversed) => Err(PayrollError::AlreadyReversed),
            Err(PayrollError::PostingNotFound) => Err(PayrollError::PostingNotFound),
            Err(error) => {
                self.failed(scope, company, identity(&original), true, error)
                    .await
            }
        }
    }

    async fn reverse_once(
        &self,
        scope: &Scope,
        company: i64,
        id: i64,
        date: Option<Date>,
        why: &str,
    ) -> Result<Posting> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .ledger
            .write(
                &mut client,
                scope,
                company,
                &[POST_POSTINGS],
                "finance.payroll.reverse",
            )
            .await
            .map_err(ledger_error)?;
        let post = store::posting(tx.sql(), scope.tenant(), company, id).await?;
        if post.reversal.is_some() {
            return Err(PayrollError::AlreadyReversed);
        }
        let date = match date {
            Some(date) => date,
            None => {
                let timezone = self
                    .settings
                    .get(
                        tx.sql(),
                        scope,
                        Target::Company(company.try_into().map_err(|_| Error::Invalid)?),
                        bilibi_datetime::TIMEZONE_KEY,
                    )
                    .await?;
                let zone = TimeZone::get(timezone.value.as_str().ok_or(Error::Internal)?)
                    .map_err(|_| Error::Internal)?;
                Timestamp::now().to_zoned(zone).date()
            }
        };
        let entry = self
            .ledger
            .reverse_owned_entry(
                &mut tx,
                scope,
                company,
                post.entry_id,
                &owner(&identity(&post)),
                &bilibi_finance_ledger::Reversal {
                    entry_date: date,
                    description: format!(
                        "Reversal of {} {}",
                        if post.source_kind == "payroll_run" {
                            "Payroll run"
                        } else {
                            "Staff"
                        },
                        post.source_reference
                    ),
                    reference: Some(
                        format!(
                            "{} {}: {}",
                            post.source_kind, post.source_id, post.source_reference
                        )
                        .chars()
                        .take(100)
                        .collect(),
                    ),
                },
            )
            .await?;
        let amount = entry.lines.iter().map(|l| l.debit).sum();
        store::record(
            &mut tx,
            scope,
            company,
            identity(&post),
            store::Outcome::Reversed(&entry, amount, why),
        )
        .await?;
        let post = store::posting(tx.sql(), scope.tenant(), company, id).await?;
        tx.commit().await?;
        Ok(post)
    }

    async fn committed(
        &self,
        scope: &Scope,
        company: i64,
        source: store::Source<'_>,
        reverse: bool,
    ) -> Result<Option<Posting>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company, POST_POSTINGS).await?;
        let row = client.query_opt("SELECT id FROM finance_payroll_posting_events WHERE tenant_id=$1 AND company_id=$2 AND source_kind=$3 AND source_id=$4 AND event='posted'", &[&scope.tenant(),&company,&source.kind,&source.id]).await?;
        match row {
            Some(r) => {
                let p = store::posting(&**client, scope.tenant(), company, r.get(0)).await?;
                Ok((!reverse || p.reversal.is_some()).then_some(p))
            }
            None => Ok(None),
        }
    }

    async fn failed(
        &self,
        scope: &Scope,
        company: i64,
        source: store::Source<'_>,
        reverse: bool,
        error: PayrollError,
    ) -> Result<Posting> {
        // Infrastructure faults that are not the explicitly retryable class
        // remain infrastructure faults, not invented business refusals.
        if infrastructure(&error) && !transient(&error) {
            return Err(error);
        }
        if transient(&error) {
            if let Some(post) = self
                .committed(scope, company, store::Source { ..source }, reverse)
                .await?
            {
                return Ok(post);
            }
        }
        let message = message(&error).chars().take(500).collect::<String>();
        // The failed transaction has already dropped/rolled back. A new
        // authorized write records the failure; unwritable/unreachable means
        // best effort, never a success event without its ledger entry.
        if let Ok(mut client) = self.db.get().await {
            if let Ok(mut tx) = self
                .ledger
                .write(
                    &mut client,
                    scope,
                    company,
                    &[POST_POSTINGS],
                    "finance.payroll.attempt_failed",
                )
                .await
            {
                // A competing retry may have committed after our failure.
                // Never let a stale failure hide that newer success.
                if !succeeded(&tx, scope, company, &source, reverse)
                    .await
                    .unwrap_or(true)
                    && store::record(
                        &mut tx,
                        scope,
                        company,
                        source,
                        store::Outcome::Failed {
                            reverse,
                            reason: &message,
                        },
                    )
                    .await
                    .is_ok()
                {
                    let _ = tx.commit().await;
                }
            }
        }
        Err(PayrollError::AttemptFailed(message))
    }
}

async fn succeeded(
    tx: &MutationTransaction<'_>,
    scope: &Scope,
    company: i64,
    source: &store::Source<'_>,
    reverse: bool,
) -> Result<bool> {
    if !reverse {
        return store::already_posted(tx.sql(), scope.tenant(), company, source.kind, source.id)
            .await;
    }
    Ok(tx.sql().query_opt("SELECT 1 FROM finance_payroll_posting_events WHERE tenant_id=$1 AND company_id=$2 AND source_kind=$3 AND source_id=$4 AND event='reversed'", &[&scope.tenant(),&company,&source.kind,&source.id]).await?.is_some())
}
fn identity(p: &Posting) -> store::Source<'_> {
    store::Source {
        kind: &p.source_kind,
        id: p.source_id,
        reference: &p.source_reference,
    }
}
fn owner(s: &store::Source<'_>) -> EntryOwner {
    EntryOwner {
        module: MODULE.into(),
        record: format!("{}:{}", s.kind, s.id),
    }
}
fn failed(message: impl Into<String>) -> PayrollError {
    PayrollError::AttemptFailed(message.into())
}
fn infrastructure(error: &PayrollError) -> bool {
    matches!(
        error,
        PayrollError::Failed(_) | PayrollError::Ledger(LedgerError::Failed(_))
    )
}
fn transient(error: &PayrollError) -> bool {
    // Transparent error wrappers forward their inner source(), skipping the
    // database enum itself. Match pool failures before traversing the chain.
    if matches!(
        error,
        PayrollError::Failed(Error::Pool(_))
            | PayrollError::Ledger(LedgerError::Failed(Error::Pool(_)))
    ) {
        return true;
    }
    let mut cause: Option<&(dyn StdError + 'static)> = Some(error);
    while let Some(error) = cause {
        if let Some(db) = error.downcast_ref::<tokio_postgres::error::DbError>() {
            return matches!(db.code().code(), "55P03" | "40001" | "40P01" | "57014");
        }
        if error.is::<std::io::Error>() {
            return true;
        }
        if let Some(Error::Pool(_)) = error.downcast_ref::<Error>() {
            return true;
        }
        cause = error.source();
    }
    false
}
fn message(error: &PayrollError) -> String {
    match error {
        PayrollError::AttemptFailed(reason) => reason.clone(),
        PayrollError::Ledger(LedgerError::Invalid(errors)) | PayrollError::Invalid(errors) => {
            errors
                .iter()
                .map(|e| match e.line {
                    Some(n) => format!("lines[{n}].{} {}", e.field.replace('_', " "), e.message),
                    None => format!("{} {}", e.field.replace('_', " "), e.message),
                })
                .collect::<Vec<_>>()
                .join("; ")
        }
        PayrollError::Ledger(LedgerError::Unauthorized) => {
            "the actor may not post to the ledger".into()
        }
        PayrollError::Ledger(LedgerError::BookNotOpen) => "the company's book is not open".into(),
        PayrollError::CompanyArchived => "the company is archived".into(),
        PayrollError::CompanyNotFound => "the company could not be found".into(),
        PayrollError::PeopleUnavailable => "the payroll source is not installed".into(),
        PayrollError::SourceNotFound => "the source no longer exists".into(),
        PayrollError::Ledger(LedgerError::EntryNotFound) => {
            "the ledger entry no longer exists".into()
        }
        PayrollError::Ledger(LedgerError::AlreadyReversed) => {
            "the ledger entry is already reversed".into()
        }
        other if transient(other) => format!("a database failure, tried twice: {other}"),
        other => other.to_string(),
    }
}

fn rounded(amount: Decimal, book: &Book) -> Result<Money> {
    Money::new(
        amount.round_dp_with_strategy(
            u32::from(book.minor_units.get()),
            RoundingStrategy::MidpointAwayFromZero,
        ),
        book.minor_units,
    )
    .map_err(|e| failed(e.to_string()))
}
fn total(lines: &[NewLine], units: bilibi_finance_ledger::MinorUnits) -> Result<Money> {
    Money::new(lines.iter().map(|l| l.debit.amount()).sum(), units)
        .map_err(|e| failed(e.to_string()))
}

fn build(
    document: &Document,
    book: &Book,
    settings: &Settings,
    mappings: &[Mapping],
) -> Result<NewEntry> {
    let source = document.source();
    let (date, description, lines) = match document {
        Document::Run(run) => {
            if !run.finalised {
                return Err(failed("the run is not finalised"));
            }
            let date = run
                .pay_date
                .ok_or_else(|| failed("the run's pay date is missing or invalid"))?;
            let mut parts: BTreeMap<(&str, &str), Decimal> = BTreeMap::new();
            for c in &run.components {
                let amount = parts.entry((&c.code, &c.direction)).or_default();
                *amount = amount
                    .checked_add(c.amount)
                    .ok_or_else(|| failed("amount must be less than 1E+16"))?;
            }
            let mut missing = Vec::new();
            let mut mapped = Vec::new();
            for ((code, direction), amount) in parts {
                match mappings
                    .iter()
                    .find(|m| m.kind == "component" && m.code == code && m.direction == direction)
                {
                    None => missing.push(format!("{code} ({direction})")),
                    Some(mapping) => mapped.push((code, direction, amount, mapping)),
                }
            }
            if !missing.is_empty() {
                return Err(failed(format!(
                    "components not mapped to accounts: {}",
                    missing.join(", ")
                )));
            }
            let mut lines = Vec::new();
            let mut net = Decimal::ZERO;
            for (code, direction, amount, mapping) in mapped {
                let amount = rounded(amount, book)?;
                if amount.is_zero() {
                    continue;
                }
                let description = format!("{code} ({direction})");
                if matches!(direction, "earning" | "employer") {
                    lines.push(
                        NewLine::debit(
                            mapping.expense_account_id.expect("validated mapping"),
                            amount,
                        )
                        .described(&description),
                    );
                }
                if matches!(direction, "deduction" | "employer") {
                    lines.push(
                        NewLine::credit(
                            mapping.liability_account_id.expect("validated mapping"),
                            amount,
                        )
                        .described(&description),
                    );
                }
                net = match direction {
                    "earning" => net.checked_add(amount.amount()),
                    "deduction" => net.checked_sub(amount.amount()),
                    _ => Some(net),
                }
                .ok_or_else(|| failed("amount must be less than 1E+16"))?;
            }
            let net = Money::new(net, book.minor_units).map_err(|e| failed(e.to_string()))?;
            if !net.is_zero() {
                lines.push(
                    if net.is_positive() {
                        NewLine::credit(settings.net_pay_account_id, net)
                    } else {
                        NewLine::debit(settings.net_pay_account_id, net.abs())
                    }
                    .described("Net pay"),
                );
            }
            if lines.len() < 2 {
                return Err(failed("the run's amounts are zero"));
            }
            (date, format!("Payroll run {}", run.reference), lines)
        }
        Document::Claim(claim) => {
            let date = claim
                .approved_on
                .ok_or_else(|| failed("the claim's approval date could not be worked out"))?;
            let category = claim
                .category_code
                .as_ref()
                .ok_or_else(|| failed("the claim's category is unknown"))?;
            let mapping = mappings
                .iter()
                .find(|m| m.kind == "claim_category" && m.code == *category)
                .ok_or_else(|| {
                    failed(format!(
                        "claim category not mapped to an account: {category}"
                    ))
                })?;
            let amount = rounded(claim.amount, book)?;
            if !amount.is_positive() {
                return Err(failed("the claim's amount is zero"));
            }
            (
                date,
                format!("Staff {}", claim.reference),
                vec![
                    NewLine::debit(
                        mapping.expense_account_id.expect("validated mapping"),
                        amount,
                    )
                    .described(format!("Claim category {category}")),
                    NewLine::credit(settings.claims_payable_account_id, amount)
                        .described("Owed to staff"),
                ],
            )
        }
    };
    Ok(NewEntry {
        entry_date: date,
        description,
        reference: Some(
            format!("{} {}: {}", source.kind, source.id, source.reference)
                .chars()
                .take(100)
                .collect(),
        ),
        lines,
    })
}
