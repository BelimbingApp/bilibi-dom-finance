//! The general ledger, module `finance/ledger` (Bilimbi
//! `Bilimbi.Finance.Ledger`): a company's book, chart of accounts, journal
//! entries, fiscal years and their closing, controlled bookkeeping and late
//! recordings, and in later versions its journals and the separation of
//! duties. Other Finance modules post and read through this facade; none
//! reads its tables.
//!
//! Every function takes the sealed [`Scope`] it was handed and an explicit
//! company id. Reads authorize `finance.ledger.view` and writes their own
//! capability for the scope's actor and the target company at the moment
//! they run, through Core Company's `authorize_target`: a missing grant, or a
//! sibling company without tenant-wide reach, is
//! [`LedgerError::Unauthorized`], and a company the scope cannot see is
//! [`LedgerError::CompanyNotFound`]. A write then holds the company's row
//! lock for its mutation transaction, so an archived company is
//! [`LedgerError::CompanyArchived`] and the company's ledger writes run one
//! at a time.
//!
//! A book is opened once per company with its currency code and that
//! currency's number of decimal places; both are fixed for good, because
//! every posted amount is denominated in them ([`Money`]). A new company's
//! book opens controlled (see `bookkeeping.rs`). Accounts are never deleted
//! and keep their code and type; the one equity account designated for
//! retained earnings receives the year-end close.
//!
//! A company divides its time into fiscal years of monthly periods. Periods
//! close in date order and reopen in reverse; a reopen needs a reason and the
//! reopen capability, and a locked period never reopens.
//! [`Ledger::close_fiscal_year`] posts the entry that moves the year's income
//! and expense balances into retained earnings and closes the year with its
//! periods. Every close, reopen and lock is an append-only closing event, and
//! a status is the latest event's. A controlled company posts only into a
//! period it has opened and left open; an entry is never refused for being
//! old, but one recorded 45 or more days after its date is listed for an
//! independent review. A reopen, and turning controlled bookkeeping off,
//! override a control and are recorded as approval exceptions.
//!
//! Results are plain records; the schema, SQL and validation stay private.
mod accounts;
mod bookkeeping;
mod books;
mod contributions;
mod duties;
mod entries;
mod late_recordings;
mod money;
mod periods;
mod postings;

pub use contributions::{
    APPROVE_JOURNALS, CAPABILITIES, LOCK_PERIODS, MANAGE_ACCOUNTS, MANAGE_CONTROLLED_BOOKKEEPING,
    MANAGE_DUTY_SEPARATION, MANAGE_PERIODS, POST_ENTRIES, REOPEN_PERIODS, REVIEW_EXCEPTIONS,
    REVIEW_LATE_RECORDINGS, SUBMIT_JOURNALS, VIEW,
};
pub use money::{MinorUnits, Money, MoneyError};

use bilibi_company::{organisation, WriteError};
use bilibi_database::{Change, Db, Error, MutationBoundary, MutationTransaction};
use bilibi_module_registry::{Layer, Module, Service};
use bilibi_tenancy::Scope;
use jiff::{civil::Date, Timestamp};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio_postgres::GenericClient;

/// The descriptor id, which later versions record as the owner of entries.
pub const MODULE: &str = "finance/ledger";
/// The registry service id of the [`Ledger`] facade.
pub const SERVICE: &str = "finance/ledger.ledger";

inventory::submit! {
    Module {
        id: MODULE, layer: Layer::Domains,
        dependencies: &["base/authz", "base/database", "base/tenancy", "core/company"],
        migrations: &[
            ("finance-ledger-001-books-and-accounts", include_str!("../migrations/001-books-and-accounts.sql")),
            ("finance-ledger-002-entries", include_str!("../migrations/002-entries.sql")),
            ("finance-ledger-003-posting-notices", include_str!("../migrations/003-posting-notices.sql")),
            ("finance-ledger-004-periods", include_str!("../migrations/004-periods.sql")),
        ],
        services: &[Service { id: SERVICE, factory: |services| Ok(Arc::new(Ledger {
            db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
            mutations: services.get("base/database.mutations")?,
            postings: postings::Relay::new(),
        })) }],
    }
}

pub type Result<T> = std::result::Result<T, LedgerError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// A system scope, a missing grant, or a sibling company without
    /// tenant-wide reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company the scope cannot see: missing, deleted or another tenant's.
    #[error("Not found")]
    CompanyNotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    #[error("The company's book is not open")]
    BookNotOpen,
    #[error("The company's book is already open")]
    BookAlreadyOpen,
    #[error("Not found")]
    AccountNotFound,
    #[error("Not found")]
    EntryNotFound,
    #[error("Not found")]
    FiscalYearNotFound,
    #[error("Not found")]
    PeriodNotFound,
    /// Closing a period that is closed or locked.
    #[error("The period is not open")]
    PeriodNotOpen,
    /// Reopening or locking a period that is open.
    #[error("The period is not closed")]
    PeriodNotClosed,
    /// Reopening a locked period, or a fiscal year whose last period is.
    #[error("The period is locked and never reopens")]
    PeriodLocked,
    /// Periods close in date order; so does a fiscal year after the previous one.
    #[error("An earlier period is still open")]
    EarlierPeriodOpen,
    /// A year's last period closes only with the year.
    #[error("The period closes with its fiscal year")]
    ClosesWithFiscalYear,
    /// Periods reopen in reverse date order.
    #[error("A later period is closed")]
    LaterPeriodClosed,
    /// Periods lock in date order.
    #[error("An earlier period is not locked")]
    EarlierPeriodUnlocked,
    /// A closed fiscal year's periods reopen only after the year.
    #[error("The fiscal year is closed")]
    FiscalYearClosed,
    #[error("The fiscal year is not closed")]
    FiscalYearNotClosed,
    /// The year-end close needs an active equity account designated for
    /// retained earnings.
    #[error("No retained-earnings account is designated")]
    NoRetainedEarningsAccount,
    #[error("That entry was not recorded late")]
    NotLateRecorded,
    #[error("Someone has already reviewed that entry")]
    AlreadyReviewed,
    /// The reviewer posted the entry, or impersonated (or was impersonated
    /// by) whoever did.
    #[error("An entry is reviewed by someone who did not post it")]
    ReviewerNotIndependent,
    /// An option the call does not accept: an `owner` or settlement account
    /// of another shape, or a limit, cursor or bound out of range.
    #[error("Invalid option")]
    InvalidOption,
    /// The relay's database connection could not be made to listen.
    #[error("Posting notices are unavailable")]
    NoticesUnavailable,
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(#[from] Error),
}

/// One refusal of a submitted value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
    /// The 1-based line of an entry the refusal is about, when it is about one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}
impl FieldError {
    pub fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
            line: None,
        }
    }
    pub(crate) fn on_line(mut self, line: usize) -> Self {
        self.line = Some(line);
        self
    }
}

/// The company's book: its currency and that currency's decimal places.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Book {
    pub company_id: i64,
    pub currency_code: String,
    pub minor_units: MinorUnits,
}
/// What opens a book: three letters (trimmed and upcased) and 0 to 4.
#[derive(Clone, Debug)]
pub struct NewBook {
    pub currency_code: String,
    pub minor_units: i64,
}

/// The five types every chart of accounts classifies by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}
impl AccountType {
    pub const ALL: [AccountType; 5] = [
        Self::Asset,
        Self::Liability,
        Self::Equity,
        Self::Income,
        Self::Expense,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Equity => "equity",
            Self::Income => "income",
            Self::Expense => "expense",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == value.trim())
    }
}

/// One account of the chart, as the facade shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Account {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub parent_id: Option<i64>,
    pub active: bool,
    pub retained_earnings: bool,
}
/// What creates an account: `code` (unique in the company), `name` and
/// `account_type`, optionally grouped under a `parent_id` of the same type,
/// `active` (true by default) and `retained_earnings` (false by default).
#[derive(Clone, Debug)]
pub struct NewAccount {
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub parent_id: Option<i64>,
    pub active: bool,
    pub retained_earnings: bool,
}
impl NewAccount {
    pub fn new(
        code: impl Into<String>,
        name: impl Into<String>,
        account_type: AccountType,
    ) -> Self {
        Self {
            code: code.into(),
            name: name.into(),
            account_type,
            parent_id: None,
            active: true,
            retained_earnings: false,
        }
    }
}
/// What changes on an account: each `Some` replaces the field; `parent_id:
/// Some(None)` removes the grouping. Code and type never change.
#[derive(Clone, Debug, Default)]
pub struct AccountChange {
    pub name: Option<String>,
    pub parent_id: Option<Option<i64>>,
    pub active: Option<bool>,
    pub retained_earnings: Option<bool>,
}

/// The module and record an entry belongs to: `module` is the owner's
/// descriptor id (`finance/receivables`) and `record` a kind and a positive
/// integer in the owner's own terms (`payment:17`). A manual entry has none.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EntryOwner {
    pub module: String,
    pub record: String,
}

/// One line of a new entry: an account and exactly one positive side.
#[derive(Clone, Debug)]
pub struct NewLine {
    pub account_id: i64,
    pub debit: Money,
    pub credit: Money,
    pub description: Option<String>,
}
impl NewLine {
    pub fn debit(account_id: i64, amount: Money) -> Self {
        Self {
            account_id,
            debit: amount,
            credit: Money::ZERO,
            description: None,
        }
    }
    pub fn credit(account_id: i64, amount: Money) -> Self {
        Self {
            account_id,
            debit: Money::ZERO,
            credit: amount,
            description: None,
        }
    }
    pub fn described(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}
/// What posts an entry: `entry_date`, `description` (1 to 500 characters),
/// an optional `reference` (at most 100) and at least two `lines`.
#[derive(Clone, Debug)]
pub struct NewEntry {
    pub entry_date: Date,
    pub description: String,
    pub reference: Option<String>,
    pub lines: Vec<NewLine>,
}
/// Options of [`Ledger::post_entry`] (Bilimbi's keyword options).
#[derive(Clone, Debug, Default)]
pub struct PostOptions {
    /// Accounts that may be inactive in this entry only, for a module that
    /// settles a document it posted earlier. Each must still be an account of
    /// the company; every other line still needs an active account.
    pub settlement_account_ids: Vec<i64>,
    /// The module and record this entry recognises. The record has at most
    /// one original entry in the company, and an owned entry is corrected
    /// only through its owner.
    pub owner: Option<EntryOwner>,
}

/// One line of a posted entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EntryLine {
    pub line_number: i32,
    pub account_id: i64,
    pub debit: Money,
    pub credit: Money,
    pub description: Option<String>,
}
/// A posted entry with its lines, as the database stored it. The number and
/// time are the database's; `reversed_by_entry_id` names the later entry that
/// reversed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Entry {
    pub id: i64,
    pub entry_number: i64,
    #[serde(serialize_with = "as_text")]
    pub entry_date: Date,
    pub description: String,
    pub reference: Option<String>,
    pub currency_code: String,
    pub reverses_entry_id: Option<i64>,
    pub owner: Option<EntryOwner>,
    pub posted_by_user_id: i64,
    pub posted_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub posted_at: Timestamp,
    pub reversed_by_entry_id: Option<i64>,
    pub lines: Vec<EntryLine>,
}
/// Dates and instants are written as their ISO 8601 text, so the Domain
/// does not switch on a serde feature of the shared `jiff` crate.
fn as_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}
/// Bounds of [`Ledger::list_entries`]; `Default` lists the first 100.
#[derive(Clone, Debug, Default)]
pub struct EntryQuery {
    /// Entry dates, both inclusive.
    pub from: Option<Date>,
    pub to: Option<Date>,
    /// 1 to 500, 100 when absent.
    pub limit: Option<u32>,
    /// The `(entry_date, id)` of the last entry already read: only entries
    /// after it come back.
    pub after: Option<(Date, i64)>,
    /// Entries with a line on this account.
    pub account_id: Option<i64>,
    /// Entries numbered up to this one, to reproduce an earlier read.
    pub through_entry_number: Option<i64>,
    /// Leaves out year-end closing entries and their reversals.
    pub exclude_closing: bool,
}

/// A fiscal year's status, its latest closing event's (open with none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum YearStatus {
    Open,
    Closed,
}
impl YearStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }
}
/// A period's status, its latest closing event's (open with none).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeriodStatus {
    Open,
    Closed,
    Locked,
}
impl PeriodStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Locked => "locked",
        }
    }
}
/// What a closing event does to its period, or (with no period) its year.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ClosingAction {
    Close,
    Reopen,
    Lock,
}
impl ClosingAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Close => "close",
            Self::Reopen => "reopen",
            Self::Lock => "lock",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        [Self::Close, Self::Reopen, Self::Lock]
            .into_iter()
            .find(|action| action.as_str() == value)
    }
    /// The status the action leaves a period in.
    pub fn status_after(self) -> PeriodStatus {
        match self {
            Self::Close => PeriodStatus::Closed,
            Self::Reopen => PeriodStatus::Open,
            Self::Lock => PeriodStatus::Locked,
        }
    }
}

/// What opens a fiscal year: the first day of a month and the last day of a
/// month, one to 24 months on. The start defaults to the day after the
/// previous year ends (the first year needs one), the end to twelve months
/// on.
#[derive(Clone, Debug, Default)]
pub struct NewFiscalYear {
    pub start_date: Option<Date>,
    pub end_date: Option<Date>,
}
/// One calendar month of a fiscal year.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Period {
    pub id: i64,
    pub fiscal_year_id: i64,
    pub period_number: i16,
    #[serde(serialize_with = "as_text")]
    pub start_date: Date,
    #[serde(serialize_with = "as_text")]
    pub end_date: Date,
    pub status: PeriodStatus,
}
/// A fiscal year with its periods in date order; `closing_entry_id` names the
/// year-end closing entry while the year is closed by one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FiscalYear {
    pub id: i64,
    #[serde(serialize_with = "as_text")]
    pub start_date: Date,
    #[serde(serialize_with = "as_text")]
    pub end_date: Date,
    pub opened_by_user_id: i64,
    pub opened_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub opened_at: Timestamp,
    pub status: YearStatus,
    pub closing_entry_id: Option<i64>,
    pub periods: Vec<Period>,
}
/// One event of a fiscal year's closing log; `period_id` is `None` for an
/// event on the year itself, whose close names the closing entry and the
/// retained-earnings account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ClosingEvent {
    pub event_number: i64,
    pub fiscal_year_id: i64,
    pub period_id: Option<i64>,
    pub action: ClosingAction,
    pub reason: Option<String>,
    pub entry_id: Option<i64>,
    pub retained_earnings_account_id: Option<i64>,
    pub recorded_by_user_id: i64,
    pub recorded_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub recorded_at: Timestamp,
}
/// What the year-end close may say about its entry: the description
/// defaults to `Close fiscal year <start> to <end>`.
#[derive(Clone, Debug, Default)]
pub struct YearClose {
    pub description: Option<String>,
    pub reference: Option<String>,
}

/// One change of the company's controlled bookkeeping.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BookkeepingChange {
    pub id: i64,
    pub change_number: i64,
    pub controlled: bool,
    pub reason: Option<String>,
    pub changed_by_user_id: i64,
    pub changed_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub changed_at: Timestamp,
}
/// The company's controlled bookkeeping: the setting, its latest change and
/// how many fiscal years the company has opened. A controlled company with
/// no fiscal year has no period to post into until one is opened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ControlledBookkeeping {
    pub controlled: bool,
    pub change: Option<BookkeepingChange>,
    pub fiscal_years: usize,
}

/// The late-recording report's thresholds, in days from an entry's date to
/// its recording: listed from `approaching_days`, late past `late_days`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LateRecordingThresholds {
    pub approaching_days: i32,
    pub late_days: i32,
}
/// Where a listed entry stands against the thresholds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LateStatus {
    Approaching,
    Late,
}
/// An independent reviewer's reason for a late entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LateRecordingReview {
    pub id: i64,
    pub entry_id: i64,
    pub reason: String,
    pub reviewed_by_user_id: i64,
    pub reviewed_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub reviewed_at: Timestamp,
}
/// A posted entry recorded late: `days` are whole UTC days from its
/// `entry_date` (its transaction date) to `posted_at` (its recording).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LateRecording {
    pub entry_id: i64,
    pub entry_number: i64,
    #[serde(serialize_with = "as_text")]
    pub entry_date: Date,
    #[serde(serialize_with = "as_text")]
    pub posted_at: Timestamp,
    pub days: i32,
    pub status: LateStatus,
    pub description: String,
    pub reference: Option<String>,
    pub posted_by_user_id: i64,
    pub posted_by_impersonator_id: Option<i64>,
    pub review: Option<LateRecordingReview>,
}
/// Bounds of [`Ledger::list_late_recordings`]; `Default` lists the first 100
/// of both statuses, reviewed or not.
#[derive(Clone, Debug, Default)]
pub struct LateRecordingQuery {
    pub status: Option<LateStatus>,
    pub reviewed: Option<bool>,
    /// 1 to 500, 100 when absent.
    pub limit: Option<u32>,
    /// The last `entry_number` of the previous page.
    pub after: Option<i64>,
}

/// What a [`PostingSubscription`] hears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostingEvent {
    /// The entry numbered `entry_number` committed in the subscribed company.
    Posted { company_id: i64, entry_number: i64 },
    /// Notices may have been missed (the listening connection dropped, or the
    /// subscriber fell behind): read the ledger again.
    Refresh,
}
/// A company's posting notices, from [`Ledger::subscribe_postings`]; it ends
/// when dropped.
pub struct PostingSubscription {
    tenant: i64,
    company_id: i64,
    receiver: tokio::sync::broadcast::Receiver<postings::Heard>,
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Ledger {
    db: Db,
    mutations: Arc<MutationBoundary>,
    postings: postings::Relay,
}

impl Ledger {
    /// The account types a chart of accounts classifies by.
    pub fn account_types() -> &'static [AccountType] {
        &AccountType::ALL
    }

    /// The company's book (`finance.ledger.view`), or `BookNotOpen` before
    /// [`Ledger::open_book`].
    pub async fn get_book(&self, scope: &Scope, company_id: i64) -> Result<Book> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        books::find(&**client, scope.tenant(), company_id)
            .await?
            .ok_or(LedgerError::BookNotOpen)
    }

    /// Opens the company's book once (`finance.ledger.accounts.manage`); a
    /// second call is `BookAlreadyOpen`. The new book is controlled from the
    /// start.
    pub async fn open_book(&self, scope: &Scope, company_id: i64, new: &NewBook) -> Result<Book> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_ACCOUNTS],
                "finance.ledger.open_book",
            )
            .await?;
        let tenant = scope.tenant();
        if books::find(tx.sql(), tenant, company_id).await?.is_some() {
            return Err(LedgerError::BookAlreadyOpen);
        }
        let valid = books::validate(new).map_err(LedgerError::Invalid)?;
        let controlled = bookkeeping::controlled(tx.sql(), tenant, company_id).await?;
        let actor = scope.user();
        let impersonator = scope.impersonator().map(|operator| operator.user());
        let book = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let (id, book) = books::insert(sql, tenant, company_id, &valid).await?;
                    let mut changes = vec![Change {
                        kind: "finance.ledger.book".into(),
                        id,
                        old: serde_json::Value::Null,
                        new: books::to_json(&book),
                    }];
                    if !controlled {
                        let (id, recorded) = bookkeeping::record(
                            sql,
                            tenant,
                            company_id,
                            actor,
                            impersonator,
                            true,
                            None,
                        )
                        .await?;
                        changes.push(Change {
                            kind: "finance.ledger.bookkeeping_change".into(),
                            id,
                            old: serde_json::Value::Null,
                            new: recorded,
                        });
                    }
                    Ok((book, changes))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(book)
    }

    /// The company's accounts, ordered by code (`finance.ledger.view`).
    pub async fn list_accounts(&self, scope: &Scope, company_id: i64) -> Result<Vec<Account>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        Ok(accounts::list(&**client, scope.tenant(), company_id).await?)
    }

    /// Adds an account (`finance.ledger.accounts.manage`). `retained_earnings`
    /// designates an equity account as the one the year-end close moves
    /// profit into; designating another account later moves the designation.
    pub async fn create_account(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewAccount,
    ) -> Result<Account> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_ACCOUNTS],
                "finance.ledger.create_account",
            )
            .await?;
        let tenant = scope.tenant();
        let valid = accounts::validate_new(tx.sql(), tenant, company_id, new)
            .await?
            .map_err(LedgerError::Invalid)?;
        let account = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let mut changes = Vec::new();
                    if valid.retained_earnings {
                        if let Some((id, old, new)) =
                            accounts::release_designation(sql, tenant, company_id, None).await?
                        {
                            changes.push(account_change(id, old, new));
                        }
                    }
                    let account = accounts::insert(sql, tenant, company_id, &valid).await?;
                    changes.push(account_change(
                        account.id,
                        serde_json::Value::Null,
                        accounts::to_json(&account),
                    ));
                    Ok((account, changes))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(account)
    }

    /// Changes an account's name, parent, active flag or retained-earnings
    /// designation (`finance.ledger.accounts.manage`). A deactivated account
    /// takes no new postings and is deactivated only once its balance is
    /// zero; its history stays.
    pub async fn update_account(
        &self,
        scope: &Scope,
        company_id: i64,
        account_id: i64,
        change: &AccountChange,
    ) -> Result<Account> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_ACCOUNTS],
                "finance.ledger.update_account",
            )
            .await?;
        let tenant = scope.tenant();
        let current = accounts::find(tx.sql(), tenant, company_id, account_id)
            .await?
            .ok_or(LedgerError::AccountNotFound)?;
        let units = books::find(tx.sql(), tenant, company_id)
            .await?
            .map(|book| book.minor_units)
            .unwrap_or(MinorUnits(0));
        let next = accounts::validate_change(tx.sql(), tenant, company_id, &current, change, units)
            .await?
            .map_err(LedgerError::Invalid)?;
        let account = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let mut changes = Vec::new();
                    if next.retained_earnings && !current.retained_earnings {
                        if let Some((id, old, new)) =
                            accounts::release_designation(sql, tenant, company_id, Some(current.id))
                                .await?
                        {
                            changes.push(account_change(id, old, new));
                        }
                    }
                    let saved = accounts::update(sql, tenant, company_id, &next).await?;
                    changes.push(account_change(
                        saved.id,
                        accounts::to_json(&current),
                        accounts::to_json(&saved),
                    ));
                    Ok((saved, changes))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(account)
    }

    /// Posts a journal entry (`finance.ledger.entries.post`). It is refused
    /// with [`LedgerError::Invalid`] unless it has at least two lines, its
    /// debits equal its credits, and every line names an active account of the
    /// company with exactly one positive amount no finer than the book's minor
    /// units; `BookNotOpen` before the book is opened. The database numbers
    /// the entry (1, 2, 3, ... per company, never skipping) and stamps it.
    /// `options.owner` marks the entry as the one a module's record
    /// recognises, and `settlement_account_ids` is the settlement exception to
    /// the active-account rule; an `owner` or id of another shape is
    /// `InvalidOption`. The entry's date must fall in an open period: never
    /// in a closed or locked one (`entry_date falls in a closed period`),
    /// never before the first fiscal year (`is before the first fiscal
    /// year`), and, for a controlled company, never where no period exists.
    pub async fn post_entry(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewEntry,
        options: &PostOptions,
    ) -> Result<Entry> {
        if options
            .owner
            .as_ref()
            .is_some_and(|owner| !entries::owner_valid(owner))
            || options.settlement_account_ids.iter().any(|id| *id < 1)
        {
            return Err(LedgerError::InvalidOption);
        }
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[POST_ENTRIES],
                "finance.ledger.post_entry",
            )
            .await?;
        let tenant = scope.tenant();
        let book = books::find(tx.sql(), tenant, company_id)
            .await?
            .ok_or(LedgerError::BookNotOpen)?;
        let valid = entries::validate(
            tx.sql(),
            tenant,
            company_id,
            book.minor_units,
            new,
            options.owner.as_ref(),
            entries::Inactive::Listed(&options.settlement_account_ids),
        )
        .await?
        .map_err(LedgerError::Invalid)?;
        let actor = scope.user();
        let impersonator = scope.impersonator().map(|operator| operator.user());
        let entry = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let posted = entries::insert(
                        sql,
                        tenant,
                        company_id,
                        &book.currency_code,
                        actor,
                        impersonator,
                        &valid,
                    )
                    .await?;
                    let entry = posted.entry.clone();
                    Ok((entry, entry_changes(posted)))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(entry)
    }

    /// One posted entry with its lines (`finance.ledger.view`), or
    /// `EntryNotFound` for a missing id or another company's.
    pub async fn get_entry(&self, scope: &Scope, company_id: i64, entry_id: i64) -> Result<Entry> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        entries::find(&**client, scope.tenant(), company_id, entry_id)
            .await?
            .ok_or(LedgerError::EntryNotFound)
    }

    /// Posted entries with their lines, ordered by entry date and then id
    /// (`finance.ledger.view`). A caller pages by passing the last returned
    /// entry's date and id as `after` until fewer than `limit` come back;
    /// `exclude_closing` leaves out year-end closing entries and their
    /// reversals. An out-of-range option is `InvalidOption`.
    pub async fn list_entries(
        &self,
        scope: &Scope,
        company_id: i64,
        query: &EntryQuery,
    ) -> Result<Vec<Entry>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        entries::list(&**client, scope.tenant(), company_id, query).await
    }

    /// Subscribes to the company's postings (`finance.ledger.view`). After
    /// the transaction that posts an entry commits, the subscription hears
    /// [`PostingEvent::Posted`] for it, from any process; a rolled-back post
    /// announces nothing, and a post inside a caller's own transaction is
    /// announced when that transaction commits. A notice carries no figures:
    /// read them through this facade, which authorizes the read afresh. The
    /// subscription listens before this returns, so it hears every posting
    /// committed after.
    pub async fn subscribe_postings(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<PostingSubscription> {
        {
            let client = self.db.get().await.map_err(Error::from)?;
            authorize(&**client, scope, company_id, VIEW).await?;
        }
        self.postings
            .subscribe(scope.tenant(), company_id)
            .await
            .ok_or(LedgerError::NoticesUnavailable)
    }

    // Controlled bookkeeping

    /// The company's controlled bookkeeping (`finance.ledger.view`). A company
    /// with no change is not controlled; a new company's book opens
    /// controlled.
    pub async fn get_controlled_bookkeeping(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<ControlledBookkeeping> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        controlled_view(&**client, scope.tenant(), company_id).await
    }

    /// Turns the company's controlled bookkeeping on or off
    /// (`finance.ledger.controlled-bookkeeping.manage`); turning it off needs
    /// a `reason` (`reason is required to turn this off`). The change is
    /// appended with the actor, any impersonator and the database's time; a
    /// call that changes nothing records nothing. Turning it off overrides a
    /// control, so it is also recorded as an approval exception with its
    /// reason.
    pub async fn set_controlled_bookkeeping(
        &self,
        scope: &Scope,
        company_id: i64,
        controlled: bool,
        reason: Option<&str>,
    ) -> Result<ControlledBookkeeping> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_CONTROLLED_BOOKKEEPING],
                "finance.ledger.set_controlled_bookkeeping",
            )
            .await?;
        let tenant = scope.tenant();
        let reason = bookkeeping::validate(controlled, reason).map_err(LedgerError::Invalid)?;
        if bookkeeping::controlled(tx.sql(), tenant, company_id).await? == controlled {
            let view = controlled_view(tx.sql(), tenant, company_id).await?;
            tx.rollback().await?;
            return Ok(view);
        }
        let (actor, impersonator) = people(scope);
        tx.mutate(move |sql| {
            Box::pin(async move {
                let (id, recorded) = bookkeeping::record(
                    sql,
                    tenant,
                    company_id,
                    actor,
                    impersonator,
                    controlled,
                    reason.as_deref(),
                )
                .await?;
                let mut changes = vec![Change {
                    kind: "finance.ledger.bookkeeping_change".into(),
                    id,
                    old: serde_json::Value::Null,
                    new: recorded,
                }];
                if let (false, Some(reason)) = (controlled, reason.as_deref()) {
                    changes.push(
                        duties::record_override(
                            sql,
                            tenant,
                            company_id,
                            actor,
                            impersonator,
                            &duties::Override {
                                action: "controlled_bookkeeping_off",
                                subject_type: "bookkeeping",
                                subject_id: id,
                                subject: "Controlled bookkeeping of the company".into(),
                                reason,
                            },
                        )
                        .await?,
                    );
                }
                Ok(((), changes))
            })
        })
        .await?;
        let view = controlled_view(tx.sql(), tenant, company_id).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// Every change of the company's controlled bookkeeping, oldest first
    /// (`finance.ledger.view`).
    pub async fn list_controlled_bookkeeping_changes(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<BookkeepingChange>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        Ok(bookkeeping::changes(&**client, scope.tenant(), company_id).await?)
    }

    // Late recordings

    /// The late-recording report's thresholds: listed from 45 days, late
    /// past 60.
    pub fn late_recording_thresholds() -> LateRecordingThresholds {
        LateRecordingThresholds {
            approaching_days: late_recordings::APPROACHING_DAYS,
            late_days: late_recordings::LATE_DAYS,
        }
    }

    /// Posted entries recorded 45 or more days after their date, oldest entry
    /// number first, each with its review (`finance.ledger.view`); year-end
    /// closing entries and their reversals are never listed. An out-of-range
    /// option is `InvalidOption`.
    pub async fn list_late_recordings(
        &self,
        scope: &Scope,
        company_id: i64,
        query: &LateRecordingQuery,
    ) -> Result<Vec<LateRecording>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        late_recordings::list(&**client, scope.tenant(), company_id, query).await
    }

    /// Records an independent review of a late entry with its `reason`
    /// (`finance.ledger.late-recordings.review`). The entry must be listed
    /// (`NotLateRecorded`), is reviewed once (`AlreadyReviewed`), and never by
    /// whoever posted it or impersonated its poster (`ReviewerNotIndependent`).
    pub async fn record_late_recording_review(
        &self,
        scope: &Scope,
        company_id: i64,
        entry_id: i64,
        reason: &str,
    ) -> Result<LateRecordingReview> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[REVIEW_LATE_RECORDINGS],
                "finance.ledger.record_late_recording_review",
            )
            .await?;
        let tenant = scope.tenant();
        let entry = late_recordings::entry(tx.sql(), tenant, company_id, entry_id)
            .await?
            .ok_or(LedgerError::EntryNotFound)?;
        let (actor, impersonator) = people(scope);
        if entry.days < late_recordings::APPROACHING_DAYS {
            return Err(LedgerError::NotLateRecorded);
        }
        if entry.reviewed {
            return Err(LedgerError::AlreadyReviewed);
        }
        if same_person(
            (actor, impersonator),
            (entry.posted_by_user_id, entry.posted_by_impersonator_id),
        ) {
            return Err(LedgerError::ReviewerNotIndependent);
        }
        let reason = late_recordings::validate(reason).map_err(LedgerError::Invalid)?;
        let review = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let (review, change) = late_recordings::insert_review(
                        sql,
                        tenant,
                        company_id,
                        entry_id,
                        actor,
                        impersonator,
                        &reason,
                    )
                    .await?;
                    Ok((review, vec![change]))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(review)
    }

    // Fiscal years and periods

    /// Opens the company's next fiscal year (`finance.ledger.periods.manage`)
    /// and divides it into calendar-month periods, all open. The first year
    /// may start on any month; each later year starts the day after the
    /// previous one ends. Once a company has a fiscal year, no entry is posted
    /// before the first. `BookNotOpen` before the book is opened.
    pub async fn open_fiscal_year(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewFiscalYear,
    ) -> Result<FiscalYear> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_PERIODS],
                "finance.ledger.open_fiscal_year",
            )
            .await?;
        let tenant = scope.tenant();
        books::find(tx.sql(), tenant, company_id)
            .await?
            .ok_or(LedgerError::BookNotOpen)?;
        let state = periods::state(tx.sql(), tenant, company_id).await?;
        let dates =
            periods::validate_year(state.years.last(), new).map_err(LedgerError::Invalid)?;
        let (actor, impersonator) = people(scope);
        let year_id = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    periods::insert_year(sql, tenant, company_id, actor, impersonator, dates).await
                })
            })
            .await?;
        let year = fiscal_year(tx.sql(), tenant, company_id, year_id).await?;
        tx.commit().await?;
        Ok(year)
    }

    /// The company's fiscal years in date order, each with its status, its
    /// closing entry while closed, and its periods with theirs
    /// (`finance.ledger.view`).
    pub async fn list_fiscal_years(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<FiscalYear>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        Ok(periods::state(&**client, scope.tenant(), company_id)
            .await?
            .years)
    }

    /// One fiscal year, as [`Ledger::list_fiscal_years`] shows it
    /// (`finance.ledger.view`), or `FiscalYearNotFound`.
    pub async fn get_fiscal_year(
        &self,
        scope: &Scope,
        company_id: i64,
        fiscal_year_id: i64,
    ) -> Result<FiscalYear> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        fiscal_year(&**client, scope.tenant(), company_id, fiscal_year_id).await
    }

    /// The fiscal year's closing log, oldest first: every close, reopen and
    /// lock of the year and of its periods, with its reason, who recorded it
    /// and when (`finance.ledger.view`). The company's events are numbered
    /// 1, 2, 3, ... without gaps.
    pub async fn list_closing_events(
        &self,
        scope: &Scope,
        company_id: i64,
        fiscal_year_id: i64,
    ) -> Result<Vec<ClosingEvent>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        let tenant = scope.tenant();
        fiscal_year(&**client, tenant, company_id, fiscal_year_id).await?;
        Ok(periods::events(&**client, tenant, company_id, fiscal_year_id).await?)
    }

    /// Closes an open period with an optional `reason`
    /// (`finance.ledger.periods.manage`): no entry is posted into it until it
    /// reopens. Periods close in date order (`EarlierPeriodOpen`), and a
    /// year's last period closes only with the year (`ClosesWithFiscalYear`).
    pub async fn close_period(
        &self,
        scope: &Scope,
        company_id: i64,
        period_id: i64,
        reason: Option<&str>,
    ) -> Result<Period> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_PERIODS],
                "finance.ledger.close_period",
            )
            .await?;
        let state = periods::state(tx.sql(), scope.tenant(), company_id).await?;
        let period = state.period(period_id).ok_or(LedgerError::PeriodNotFound)?;
        let (previous, _) = state.neighbours(period_id);
        if period.status != PeriodStatus::Open {
            return Err(LedgerError::PeriodNotOpen);
        }
        if previous.is_some_and(|previous| previous.status == PeriodStatus::Open) {
            return Err(LedgerError::EarlierPeriodOpen);
        }
        if state.year(period.fiscal_year_id).is_some_and(|year| {
            year.end_date == period.end_date && year.status != YearStatus::Closed
        }) {
            return Err(LedgerError::ClosesWithFiscalYear);
        }
        let reason = periods::reason(reason, None).map_err(LedgerError::Invalid)?;
        let period = period.clone();
        Self::change_period(
            scope,
            company_id,
            tx,
            period,
            ClosingAction::Close,
            reason,
            None,
        )
        .await
    }

    /// Reopens a closed period with a `reason` (`finance.ledger.periods.reopen`),
    /// so entries post into it again. Periods reopen in reverse date order
    /// (`LaterPeriodClosed`); a locked period never reopens (`PeriodLocked`),
    /// and a closed fiscal year's periods reopen only after
    /// [`Ledger::reopen_fiscal_year`] (`FiscalYearClosed`). The reopen
    /// overrides a close, so it is also recorded as an approval exception.
    pub async fn reopen_period(
        &self,
        scope: &Scope,
        company_id: i64,
        period_id: i64,
        reason: &str,
    ) -> Result<Period> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[REOPEN_PERIODS],
                "finance.ledger.reopen_period",
            )
            .await?;
        let state = periods::state(tx.sql(), scope.tenant(), company_id).await?;
        let period = state.period(period_id).ok_or(LedgerError::PeriodNotFound)?;
        let (_, next) = state.neighbours(period_id);
        match period.status {
            PeriodStatus::Locked => return Err(LedgerError::PeriodLocked),
            PeriodStatus::Open => return Err(LedgerError::PeriodNotClosed),
            PeriodStatus::Closed => {}
        }
        if state
            .year(period.fiscal_year_id)
            .is_some_and(|year| year.status == YearStatus::Closed)
        {
            return Err(LedgerError::FiscalYearClosed);
        }
        if next.is_some_and(|next| next.status != PeriodStatus::Open) {
            return Err(LedgerError::LaterPeriodClosed);
        }
        let reason = periods::reason(Some(reason), Some("is required to reopen"))
            .map_err(LedgerError::Invalid)?;
        let period = period.clone();
        let exception = duties_reopen(
            "period",
            "Period",
            period.id,
            period.start_date,
            period.end_date,
        );
        Self::change_period(
            scope,
            company_id,
            tx,
            period,
            ClosingAction::Reopen,
            reason,
            Some(exception),
        )
        .await
    }

    /// Locks a closed period for good with an optional `reason`
    /// (`finance.ledger.periods.lock`): it never reopens. Periods lock in date
    /// order (`EarlierPeriodUnlocked`).
    pub async fn lock_period(
        &self,
        scope: &Scope,
        company_id: i64,
        period_id: i64,
        reason: Option<&str>,
    ) -> Result<Period> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[LOCK_PERIODS],
                "finance.ledger.lock_period",
            )
            .await?;
        let state = periods::state(tx.sql(), scope.tenant(), company_id).await?;
        let period = state.period(period_id).ok_or(LedgerError::PeriodNotFound)?;
        let (previous, _) = state.neighbours(period_id);
        match period.status {
            PeriodStatus::Locked => return Err(LedgerError::PeriodLocked),
            PeriodStatus::Open => return Err(LedgerError::PeriodNotClosed),
            PeriodStatus::Closed => {}
        }
        if previous.is_some_and(|previous| previous.status != PeriodStatus::Locked) {
            return Err(LedgerError::EarlierPeriodUnlocked);
        }
        let reason = periods::reason(reason, None).map_err(LedgerError::Invalid)?;
        let period = period.clone();
        Self::change_period(
            scope,
            company_id,
            tx,
            period,
            ClosingAction::Lock,
            reason,
            None,
        )
        .await
    }

    /// Closes a fiscal year (`finance.ledger.periods.manage` and
    /// `finance.ledger.entries.post`). It posts the closing entry, dated the
    /// year's last day, that brings every income and expense account's
    /// balance for the year to zero against the company's retained-earnings
    /// account (`NoRetainedEarningsAccount` until an active equity account is
    /// designated), then closes the year and every period of it still open.
    /// A year with no income or expense balance closes with no entry. The
    /// previous year must be closed first (`EarlierPeriodOpen`). Closing a
    /// closed year again returns it unchanged; only
    /// [`Ledger::reopen_fiscal_year`] undoes it.
    pub async fn close_fiscal_year(
        &self,
        scope: &Scope,
        company_id: i64,
        fiscal_year_id: i64,
        close: &YearClose,
    ) -> Result<FiscalYear> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_PERIODS, POST_ENTRIES],
                "finance.ledger.close_fiscal_year",
            )
            .await?;
        let tenant = scope.tenant();
        let book = books::find(tx.sql(), tenant, company_id)
            .await?
            .ok_or(LedgerError::BookNotOpen)?;
        let state = periods::state(tx.sql(), tenant, company_id).await?;
        let year = state
            .year(fiscal_year_id)
            .ok_or(LedgerError::FiscalYearNotFound)?
            .clone();
        if year.status == YearStatus::Closed {
            tx.rollback().await?;
            return Ok(year);
        }
        let open: Vec<Period> = year
            .periods
            .iter()
            .filter(|period| period.status == PeriodStatus::Open)
            .cloned()
            .collect();
        let first_open = open
            .first()
            .map(|period| period.id)
            .ok_or(Error::Internal)?;
        if state
            .neighbours(first_open)
            .0
            .is_some_and(|previous| previous.status == PeriodStatus::Open)
        {
            return Err(LedgerError::EarlierPeriodOpen);
        }
        let account = accounts::list(tx.sql(), tenant, company_id)
            .await?
            .into_iter()
            .find(|account| account.retained_earnings && account.active)
            .ok_or(LedgerError::NoRetainedEarningsAccount)?;
        let balances = periods::closing_balances(tx.sql(), tenant, company_id, &year).await?;
        let closing = if balances.is_empty() {
            None
        } else {
            let profit_or_loss: rust_decimal::Decimal = balances.iter().map(|(_, net)| *net).sum();
            let mut lines: Vec<NewLine> =
                balances.iter().map(|(id, net)| side(*id, -*net)).collect();
            if !profit_or_loss.is_zero() {
                lines.push(side(account.id, profit_or_loss));
            }
            let new = NewEntry {
                entry_date: year.end_date,
                description: close.description.clone().unwrap_or_else(|| {
                    format!("Close fiscal year {} to {}", year.start_date, year.end_date)
                }),
                reference: close.reference.clone(),
                lines,
            };
            Some(
                entries::validate(
                    tx.sql(),
                    tenant,
                    company_id,
                    book.minor_units,
                    &new,
                    None,
                    entries::Inactive::Any,
                )
                .await?
                .map_err(LedgerError::Invalid)?,
            )
        };
        let (actor, impersonator) = people(scope);
        let currency = book.currency_code.clone();
        let account_id = account.id;
        tx.mutate(move |sql| {
            Box::pin(async move {
                let mut changes = Vec::new();
                let mut entry_id = None;
                if let Some(closing) = closing {
                    let posted = entries::insert(
                        sql,
                        tenant,
                        company_id,
                        &currency,
                        actor,
                        impersonator,
                        &closing,
                    )
                    .await?;
                    entry_id = Some(posted.entry.id);
                    changes.extend(entry_changes(posted));
                }
                let mut events = vec![periods::NewEvent {
                    fiscal_year_id,
                    period_id: None,
                    action: ClosingAction::Close,
                    reason: None,
                    entry_id,
                    retained_earnings_account_id: Some(account_id),
                }];
                events.extend(open.iter().map(|period| periods::NewEvent {
                    fiscal_year_id,
                    period_id: Some(period.id),
                    action: ClosingAction::Close,
                    reason: None,
                    entry_id: None,
                    retained_earnings_account_id: None,
                }));
                for event in &events {
                    changes.push(
                        periods::record_event(sql, tenant, company_id, actor, impersonator, event)
                            .await?,
                    );
                }
                Ok(((), changes))
            })
        })
        .await?;
        let year = fiscal_year(tx.sql(), tenant, company_id, fiscal_year_id).await?;
        tx.commit().await?;
        Ok(year)
    }

    /// Reopens a closed fiscal year with a `reason` (`finance.ledger.periods.reopen`
    /// and `finance.ledger.entries.post`): it reopens the year and its last
    /// period and reverses the closing entry on the year's last day; the
    /// year's other periods stay closed until each is reopened. A year whose
    /// last period is locked never reopens (`PeriodLocked`), and a year
    /// reopens only while the next year's periods are all open
    /// (`LaterPeriodClosed`). The reopen is recorded as an approval exception.
    pub async fn reopen_fiscal_year(
        &self,
        scope: &Scope,
        company_id: i64,
        fiscal_year_id: i64,
        reason: &str,
    ) -> Result<FiscalYear> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[REOPEN_PERIODS, POST_ENTRIES],
                "finance.ledger.reopen_fiscal_year",
            )
            .await?;
        let tenant = scope.tenant();
        let book = books::find(tx.sql(), tenant, company_id)
            .await?
            .ok_or(LedgerError::BookNotOpen)?;
        let state = periods::state(tx.sql(), tenant, company_id).await?;
        let year = state
            .year(fiscal_year_id)
            .ok_or(LedgerError::FiscalYearNotFound)?
            .clone();
        let last = year.periods.last().cloned().ok_or(Error::Internal)?;
        let (_, next) = state.neighbours(last.id);
        if year.status == YearStatus::Open {
            return Err(LedgerError::FiscalYearNotClosed);
        }
        if last.status == PeriodStatus::Locked {
            return Err(LedgerError::PeriodLocked);
        }
        if next.is_some_and(|next| next.status != PeriodStatus::Open) {
            return Err(LedgerError::LaterPeriodClosed);
        }
        let reason = periods::reason(Some(reason), Some("is required to reopen"))
            .map_err(LedgerError::Invalid)?
            .unwrap_or_default();
        let (actor, impersonator) = people(scope);
        let events_reason = reason.clone();
        tx.mutate(move |sql| {
            Box::pin(async move {
                let mut changes = Vec::new();
                for period_id in [None, Some(last.id)] {
                    changes.push(
                        periods::record_event(
                            sql,
                            tenant,
                            company_id,
                            actor,
                            impersonator,
                            &periods::NewEvent {
                                fiscal_year_id,
                                period_id,
                                action: ClosingAction::Reopen,
                                reason: Some(events_reason.clone()),
                                entry_id: None,
                                retained_earnings_account_id: None,
                            },
                        )
                        .await?,
                    );
                }
                Ok(((), changes))
            })
        })
        .await?;
        // The year's last period is open again, so its reversal is checked
        // against the closing rules as they now stand.
        let reversal = match year.closing_entry_id {
            Some(entry_id) => {
                let original = entries::find(tx.sql(), tenant, company_id, entry_id)
                    .await?
                    .ok_or(Error::Internal)?;
                let new = entries::mirror(
                    &original,
                    year.end_date,
                    format!(
                        "Reopen fiscal year {} to {}",
                        year.start_date, year.end_date
                    ),
                );
                Some(
                    entries::validate(
                        tx.sql(),
                        tenant,
                        company_id,
                        book.minor_units,
                        &new,
                        None,
                        entries::Inactive::Any,
                    )
                    .await?
                    .map_err(LedgerError::Invalid)?
                    .reversing(&original),
                )
            }
            None => None,
        };
        let exception = duties_reopen(
            "fiscal_year",
            "Fiscal year",
            year.id,
            year.start_date,
            year.end_date,
        );
        let currency = book.currency_code.clone();
        tx.mutate(move |sql| {
            Box::pin(async move {
                let mut changes = Vec::new();
                if let Some(reversal) = reversal {
                    let posted = entries::insert(
                        sql,
                        tenant,
                        company_id,
                        &currency,
                        actor,
                        impersonator,
                        &reversal,
                    )
                    .await?;
                    changes.extend(entry_changes(posted));
                }
                changes.push(
                    duties::record_override(
                        sql,
                        tenant,
                        company_id,
                        actor,
                        impersonator,
                        &exception.with_reason(&reason),
                    )
                    .await?,
                );
                Ok(((), changes))
            })
        })
        .await?;
        let year = fiscal_year(tx.sql(), tenant, company_id, fiscal_year_id).await?;
        tx.commit().await?;
        Ok(year)
    }

    /// Records the period's event (and, for a reopen, its approval
    /// exception) and returns the period with the status it now has.
    async fn change_period(
        scope: &Scope,
        company_id: i64,
        mut tx: MutationTransaction<'_>,
        period: Period,
        action: ClosingAction,
        reason: Option<String>,
        exception: Option<Reopen>,
    ) -> Result<Period> {
        let tenant = scope.tenant();
        let (actor, impersonator) = people(scope);
        let event = periods::NewEvent {
            fiscal_year_id: period.fiscal_year_id,
            period_id: Some(period.id),
            action,
            reason,
            entry_id: None,
            retained_earnings_account_id: None,
        };
        let changed = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    let mut changes = vec![
                        periods::record_event(sql, tenant, company_id, actor, impersonator, &event)
                            .await?,
                    ];
                    if let (Some(exception), Some(reason)) = (exception, event.reason.as_deref()) {
                        changes.push(
                            duties::record_override(
                                sql,
                                tenant,
                                company_id,
                                actor,
                                impersonator,
                                &exception.with_reason(reason),
                            )
                            .await?,
                        );
                    }
                    Ok((
                        Period {
                            status: action.status_after(),
                            ..period
                        },
                        changes,
                    ))
                })
            })
            .await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// Every write: each capability and company reach now, then the audited
    /// transaction holding the company's row lock (Bilimbi `write/4` with
    /// `lock_writable_company/2`).
    async fn write<'c>(
        &self,
        client: &'c mut tokio_postgres::Client,
        scope: &Scope,
        company_id: i64,
        capabilities: &[&str],
        action: &str,
    ) -> Result<MutationTransaction<'c>> {
        for capability in capabilities {
            authorize(&*client, scope, company_id, capability).await?;
        }
        let tx = self
            .mutations
            .begin(client, scope.mutation_context(action))
            .await?;
        organisation::require_writable(tx.sql(), scope, company_id, true)
            .await
            .map_err(|error| match error {
                WriteError::NotFound => LedgerError::CompanyNotFound,
                WriteError::Archived => LedgerError::CompanyArchived,
                WriteError::Access(error) => refusal(error),
                WriteError::InUse | WriteError::Stale | WriteError::Invalid(_) => {
                    LedgerError::Failed(Error::Internal)
                }
            })?;
        Ok(tx)
    }
}

/// The actor's current grant and company reach decide every call.
async fn authorize(
    client: &impl GenericClient,
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<()> {
    organisation::authorize_target(client, scope, company_id, capability)
        .await
        .map(|_| ())
        .map_err(refusal)
}

fn refusal(error: Error) -> LedgerError {
    match error {
        Error::Forbidden | Error::Unauthorized => LedgerError::Unauthorized,
        Error::NotFound => LedgerError::CompanyNotFound,
        other => LedgerError::Failed(other),
    }
}

fn account_change(id: i64, old: serde_json::Value, new: serde_json::Value) -> Change {
    Change {
        kind: "finance.ledger.account".into(),
        id,
        old,
        new,
    }
}

/// The entry and its lines as the audit records them.
fn entry_changes(posted: entries::Inserted) -> Vec<Change> {
    let mut changes = vec![Change {
        kind: "finance.ledger.entry".into(),
        id: posted.entry.id,
        old: serde_json::Value::Null,
        new: posted.entry_json,
    }];
    changes.extend(posted.lines.into_iter().map(|(id, json)| Change {
        kind: "finance.ledger.entry_line".into(),
        id,
        old: serde_json::Value::Null,
        new: json,
    }));
    changes
}

/// The scope's actor and any impersonator, as every record names them.
fn people(scope: &Scope) -> (i64, Option<i64>) {
    (
        scope.user(),
        scope.impersonator().map(|operator| operator.user()),
    )
}

/// Whether two `(user, impersonator)` people overlap: either one's user or
/// impersonator is the other's user or impersonator.
fn same_person(one: (i64, Option<i64>), other: (i64, Option<i64>)) -> bool {
    let mine = [Some(one.0), one.1];
    let theirs = [Some(other.0), other.1];
    mine.iter()
        .flatten()
        .any(|person| theirs.iter().flatten().any(|other| other == person))
}

/// A positive net is a debit, a negative one a credit.
fn side(account_id: i64, net: rust_decimal::Decimal) -> NewLine {
    if net.is_sign_positive() {
        NewLine::debit(account_id, Money::stored(net))
    } else {
        NewLine::credit(account_id, Money::stored(-net))
    }
}

async fn fiscal_year(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
    fiscal_year_id: i64,
) -> Result<FiscalYear> {
    periods::state(client, tenant, company_id)
        .await?
        .years
        .into_iter()
        .find(|year| year.id == fiscal_year_id)
        .ok_or(LedgerError::FiscalYearNotFound)
}

async fn controlled_view(
    client: &impl GenericClient,
    tenant: i64,
    company_id: i64,
) -> Result<ControlledBookkeeping> {
    let change = bookkeeping::latest(client, tenant, company_id).await?;
    Ok(ControlledBookkeeping {
        controlled: change.as_ref().is_some_and(|change| change.controlled),
        change,
        fiscal_years: periods::state(client, tenant, company_id)
            .await?
            .years
            .len(),
    })
}

/// The approval exception a reopen records: it overrides a close.
struct Reopen {
    action: &'static str,
    subject_type: &'static str,
    subject_id: i64,
    subject: String,
}
impl Reopen {
    fn with_reason<'a>(&self, reason: &'a str) -> duties::Override<'a> {
        duties::Override {
            action: self.action,
            subject_type: self.subject_type,
            subject_id: self.subject_id,
            subject: self.subject.clone(),
            reason,
        }
    }
}
fn duties_reopen(
    subject_type: &'static str,
    label: &str,
    subject_id: i64,
    start: Date,
    end: Date,
) -> Reopen {
    Reopen {
        action: if subject_type == "period" {
            "period_reopen"
        } else {
            "fiscal_year_reopen"
        },
        subject_type,
        subject_id,
        subject: format!("{label} {start} to {end}"),
    }
}
