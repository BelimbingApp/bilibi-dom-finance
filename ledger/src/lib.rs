//! The general ledger, module `finance/ledger` (Bilimbi
//! `Bilimbi.Finance.Ledger`): a company's book and chart of accounts, and in
//! later versions its journal entries, periods, journals and controls. Other
//! Finance modules post and read through this facade; none reads its tables.
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
//! Results are plain records; the schema, SQL and validation stay private.
mod accounts;
mod bookkeeping;
mod books;
mod contributions;
mod entries;
mod money;
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
                MANAGE_ACCOUNTS,
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
                MANAGE_ACCOUNTS,
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
                MANAGE_ACCOUNTS,
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
    /// `InvalidOption`.
    // TODO(finance/ledger periods): refuse an entry dated in a closed or
    // locked period, before the first fiscal year, or (for a controlled
    // company) in no fiscal period, once periods land.
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
                POST_ENTRIES,
                "finance.ledger.post_entry",
            )
            .await?;
        let tenant = scope.tenant();
        let book = books::find(tx.sql(), tenant, company_id)
            .await?
            .ok_or(LedgerError::BookNotOpen)?;
        let valid = entries::validate(tx.sql(), tenant, company_id, book.minor_units, new, options)
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
                    Ok((posted.entry, changes))
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
    /// entry's date and id as `after` until fewer than `limit` come back. An
    /// out-of-range option is `InvalidOption`.
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

    /// Every write: the capability and company reach now, then the audited
    /// transaction holding the company's row lock (Bilimbi `write/4` with
    /// `lock_writable_company/2`).
    async fn write<'c>(
        &self,
        client: &'c mut tokio_postgres::Client,
        scope: &Scope,
        company_id: i64,
        capability: &str,
        action: &str,
    ) -> Result<MutationTransaction<'c>> {
        authorize(&*client, scope, company_id, capability).await?;
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
