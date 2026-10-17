//! Bank reconciliation, module `finance/reconciliation` (Bilimbi
//! `Bilimbi.Finance.Reconciliation`): a company's bank accounts, each mapped
//! to one ledger account and carrying the import mapping its bank's CSV export
//! is read through, and the bank statements imported for them. Matching and
//! reconciling follow in later slices; the whole schema is in place.
//!
//! Every function takes the sealed [`Scope`] it was handed and an explicit
//! company id. Reads authorize `finance.reconciliation.view` and writes their
//! own capability for the scope's actor and the target company at the moment
//! they run, through Core Company's `authorize_target`: a missing grant, or a
//! sibling company without tenant-wide reach, is
//! [`ReconciliationError::Unauthorized`], and a company the scope cannot see
//! is [`ReconciliationError::CompanyNotFound`]. A write then holds the
//! company's row lock for its mutation transaction, so an archived company is
//! [`ReconciliationError::CompanyArchived`] and the company's reconciliation
//! writes run one at a time.
//!
//! The ledger is read only through `finance/ledger`'s facade with the same
//! scope, so a call that reads the chart (adding or changing a bank account)
//! or the book (importing a statement) also needs `finance.ledger.view`, and
//! the file an import keeps needs `finance.attachments.create`: each facade
//! authorizes its own calls.
//!
//! A bank account names an active asset account of the ledger that no other
//! bank account maps, and keeps it for good. Its name, counter accounts, match
//! window, import mapping and active flag change in place, and every change
//! is appended to its change log with the field, the values before and after,
//! the actor, the reason and the database's time. A bank account is never
//! deleted; deactivate it, and it imports and reconciles no more.
//!
//! A statement and its lines are inserted in one transaction, together with
//! the file itself kept whole as the statement's source document through
//! `finance/attachments` ([`StatementSource`]), and are never changed or
//! deleted. The same file (by SHA-256) and the same line (by fingerprint) are
//! refused a second time; PostgreSQL refuses them on its own as well.
//!
//! Results are plain records; the schema, SQL and validation stay private.
mod bank_accounts;
mod contributions;
mod csv;
mod evidence;
mod import;
mod mapping;
mod opening;
mod source;
mod statements;
mod web;

pub use contributions::{
    CAPABILITIES, FINALISE_RECONCILIATIONS, IMPORT_STATEMENTS, MANAGE_BANK_ACCOUNTS,
    MANAGE_MATCHES, MANAGE_RECONCILIATIONS, MANAGE_STATEMENT_LINES, POST_STATEMENT_LINES, VIEW,
};
pub use evidence::EvidencePage;
pub use import::MAX_LINES;
pub use mapping::{
    Column, ImportMapping, AMOUNT_SIGNS, DATE_FORMATS, DECIMAL_SEPARATORS, DELIMITERS,
};
pub use opening::{NewReconciliation, ReconciliationRecord};
pub use source::STATEMENT_SOURCE;

use bilibi_company::organisation;
use bilibi_database::{Db, Error, MutationTransaction};
use bilibi_finance_attachments::{AttachmentError, Attachments, NewFile};
use bilibi_finance_ledger::{Ledger, LedgerError, Money};
use bilibi_module_registry::{Layer, Module, Service};
use bilibi_tenancy::Scope;
use bilibi_ui::page::PageService;
use jiff::{civil::Date, Timestamp};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// The descriptor id.
pub const MODULE: &str = "finance/reconciliation";
/// The registry service id of the [`Reconciliation`] facade.
pub const SERVICE: &str = "finance/reconciliation.reconciliation";

inventory::submit! {
    Module {
        id: MODULE, layer: Layer::Domains,
        dependencies: &[
            "base/authz", "base/database", "base/datetime", "base/locale", "base/menu", "base/tenancy", "base/ui", "core/company", "finance/attachments", "finance/ledger",
        ],
        migrations: &[("finance-reconciliation-001-reconciliation", include_str!("../migrations/001-reconciliation.sql"))],
        services: &[Service { id: SERVICE, factory: |services| Ok(Arc::new(Reconciliation {
            db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
            ledger: services.get(bilibi_finance_ledger::SERVICE)?,
            attachments: services.get(bilibi_finance_attachments::SERVICE)?,
        })) }, Service { id: web::PAGES, factory: |services| Ok(Arc::new(PageService::<Scope, Error>(Arc::new(web::Pages {
            db: services.get::<Db>("base/database.pool")?.as_ref().clone(),
            reconciliation: services.get(SERVICE)?,
            ledger: services.get(bilibi_finance_ledger::SERVICE)?,
            attachments: services.get(bilibi_finance_attachments::SERVICE)?,
            display: services.get("base/datetime.display")?,
            locales: services.get("base/locale.locales")?,
        })))) }],
    }
}

pub type Result<T> = std::result::Result<T, ReconciliationError>;

/// Why a call did not happen (Bilimbi's refusal atoms and changeset errors).
#[derive(Debug, thiserror::Error)]
pub enum ReconciliationError {
    /// A missing grant, or a sibling company without tenant-wide reach.
    #[error("Access denied")]
    Unauthorized,
    /// A company the scope cannot see: missing, deleted or another tenant's.
    #[error("Not found")]
    CompanyNotFound,
    #[error("This company is archived and read-only")]
    CompanyArchived,
    #[error("The company's book is not open")]
    BookNotOpen,
    #[error("Not found")]
    BankAccountNotFound,
    /// An inactive bank account imports and reconciles no more.
    #[error("The bank account is inactive")]
    BankAccountInactive,
    #[error("Not found")]
    StatementNotFound,
    /// A limit or cursor out of range.
    #[error("Invalid option")]
    InvalidOption,
    /// Refused values, in the words a form shows beside each field.
    #[error("Invalid input")]
    Invalid(Vec<FieldError>),
    #[error("The file needs a name of at most 255 characters")]
    InvalidFileName,
    /// The same file (by SHA-256) is already statement `statement_number`.
    #[error("This file is already imported as statement {statement_number}")]
    DuplicateFile { statement_number: i64 },
    /// Lines the bank account already holds, with where.
    #[error("The file holds lines that are already imported")]
    DuplicateLines(Vec<DuplicateLine>),
    /// Rows the mapping cannot read, every one with why.
    #[error("The file holds rows the import mapping cannot read")]
    InvalidLines(Vec<LineError>),
    /// Header names the mapping needs that the file does not have.
    #[error("The file lacks the mapped columns: {}", .0.join(", "))]
    MissingHeaders(Vec<String>),
    #[error("The file holds no lines")]
    NoLines,
    #[error("A statement holds at most {0} lines")]
    TooManyLines(usize),
    /// The balance column does not follow from the amounts in either order.
    #[error("The running balance does not follow from the amounts at row {0}")]
    RunningBalanceMismatch(usize),
    /// The reader accepts files of at most 5 MiB.
    #[error("The file is larger than 5 MiB")]
    FileTooLarge,
    #[error("The file is not UTF-8 text")]
    NotUtf8,
    #[error("A quote in the middle of a field at row {0}")]
    StrayQuote(usize),
    #[error("A quote left open at row {0}")]
    UnterminatedQuote(usize),
    /// Source document storage refused the file or the link (an operator has
    /// not configured Base Artifacts' storage root, the file is over its
    /// limit, ...).
    #[error(transparent)]
    Attachment(AttachmentError),
    /// The ledger refused the read of its chart or book.
    #[error(transparent)]
    Ledger(LedgerError),
    /// A database or capture failure; never a refusal.
    #[error(transparent)]
    Failed(#[from] Error),
}

impl From<LedgerError> for ReconciliationError {
    fn from(error: LedgerError) -> Self {
        match error {
            LedgerError::Unauthorized => Self::Unauthorized,
            LedgerError::CompanyNotFound => Self::CompanyNotFound,
            LedgerError::CompanyArchived => Self::CompanyArchived,
            LedgerError::BookNotOpen => Self::BookNotOpen,
            LedgerError::Failed(error) => Self::Failed(error),
            other => Self::Ledger(other),
        }
    }
}
impl From<AttachmentError> for ReconciliationError {
    fn from(error: AttachmentError) -> Self {
        match error {
            AttachmentError::Unauthorized => Self::Unauthorized,
            AttachmentError::CompanyNotFound => Self::CompanyNotFound,
            AttachmentError::CompanyArchived => Self::CompanyArchived,
            AttachmentError::Failed(error) => Self::Failed(error),
            other => Self::Attachment(other),
        }
    }
}

/// One refusal of a submitted value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FieldError {
    pub field: &'static str,
    pub message: String,
}
impl FieldError {
    pub(crate) fn new(field: &'static str, message: impl Into<String>) -> Self {
        Self {
            field,
            message: message.into(),
        }
    }
}

/// A row of the file the import mapping cannot read: its 1-based number among
/// the file's non-blank rows, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LineError {
    pub row: usize,
    pub message: String,
}

/// A line of the file the bank account already holds: the file's row, the
/// line's number in this import, and the statement and line that hold it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DuplicateLine {
    pub row: usize,
    pub line_number: i32,
    pub statement_number: i64,
    pub statement_line_number: i32,
}

/// One bank account, as the facade shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BankAccount {
    pub id: i64,
    pub name: String,
    /// The active asset account of the ledger it is mapped to, for good.
    pub ledger_account_id: i64,
    /// The expense account bank charges post to.
    pub charges_account_id: Option<i64>,
    /// The income account interest posts to.
    pub interest_account_id: Option<i64>,
    /// How many days a suggested match's dates may differ by, 0 to 90.
    pub match_window_days: i32,
    pub import_mapping: ImportMapping,
    pub active: bool,
}

/// What adds a bank account: a `name` of at most 200 characters, the
/// `ledger_account_id` (an active asset account of the company no other bank
/// account maps), the `import_mapping` (see [`ImportMapping::parse`]), and
/// optionally a `charges_account_id` (an expense account), an
/// `interest_account_id` (an income account), `match_window_days` (0 to 90,
/// 5 when absent), `active` (true when absent) and a `reason` for the change
/// log.
#[derive(Clone, Debug)]
pub struct NewBankAccount {
    pub name: String,
    pub ledger_account_id: i64,
    pub import_mapping: Value,
    pub charges_account_id: Option<i64>,
    pub interest_account_id: Option<i64>,
    pub match_window_days: Option<i32>,
    pub active: Option<bool>,
    pub reason: Option<String>,
}

/// What changes on a bank account: each `Some` replaces the field;
/// `charges_account_id: Some(None)` removes the counter account. The ledger
/// account never changes.
#[derive(Clone, Debug, Default)]
pub struct BankAccountEdit {
    pub name: Option<String>,
    pub charges_account_id: Option<Option<i64>>,
    pub interest_account_id: Option<Option<i64>>,
    pub match_window_days: Option<i32>,
    pub import_mapping: Option<Value>,
    pub active: Option<bool>,
    pub reason: Option<String>,
}

/// One field of a bank account having been set or changed: the values before
/// (none for a new account) and after as text, who, why, and when.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BankAccountChange {
    pub id: i64,
    pub field: String,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub reason: Option<String>,
    pub changed_by_user_id: i64,
    pub changed_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub changed_at: Timestamp,
}

/// An imported statement: the file's name and SHA-256, the mapping it was
/// read through, its dates and balances (when the file has a running balance
/// column), and who imported it when. [`Reconciliation::get_statement`] also
/// returns its lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Statement {
    pub id: i64,
    pub bank_account_id: i64,
    pub statement_number: i64,
    pub file_name: String,
    pub file_sha256: String,
    pub import_mapping: ImportMapping,
    pub line_count: i32,
    #[serde(serialize_with = "as_text")]
    pub start_date: Date,
    #[serde(serialize_with = "as_text")]
    pub end_date: Date,
    pub opening_balance: Option<Money>,
    pub closing_balance: Option<Money>,
    pub imported_by_user_id: i64,
    pub imported_by_impersonator_id: Option<i64>,
    #[serde(serialize_with = "as_text")]
    pub imported_at: Timestamp,
    /// In line order; empty in a list.
    pub lines: Vec<StatementLine>,
}

/// One line of a statement. `amount` is signed from the account holder's
/// side: money in positive and money out negative, which is the sign of a
/// debit less a credit on the asset account it is matched against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct StatementLine {
    pub id: i64,
    pub statement_id: i64,
    pub bank_account_id: i64,
    pub line_number: i32,
    #[serde(serialize_with = "as_text")]
    pub line_date: Date,
    pub description: String,
    pub reference: Option<String>,
    pub amount: Money,
    pub running_balance: Option<Money>,
}

/// What [`Reconciliation::import_statement`] recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ImportedStatement {
    pub statement: Statement,
    /// Rows whose amount was zero, which are not lines.
    pub skipped_count: usize,
}

/// `limit` (1 to 500, 100 when absent) and `after`, the last id already read.
#[derive(Clone, Debug, Default)]
pub struct PageOptions {
    pub limit: Option<i64>,
    pub after: Option<i64>,
}
impl PageOptions {
    /// The page this asks for, or `InvalidOption`.
    pub(crate) fn resolve(&self) -> Result<(i64, i64)> {
        let limit = self.limit.unwrap_or(100);
        let after = self.after.unwrap_or(0);
        if (1..=500).contains(&limit) && after >= 0 && (self.after.is_none() || after >= 1) {
            Ok((limit, after))
        } else {
            Err(ReconciliationError::InvalidOption)
        }
    }
}

/// The choices an import mapping is built from.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct MappingChoices {
    pub delimiters: &'static [&'static str],
    pub date_formats: &'static [&'static str],
    pub amount_signs: &'static [&'static str],
    pub decimal_separators: &'static [&'static str],
}

/// The facade, resolved from the registry as [`SERVICE`].
pub struct Reconciliation {
    db: Db,
    ledger: Arc<Ledger>,
    attachments: Arc<Attachments>,
}

impl Reconciliation {
    /// The choices an import mapping is built from (see [`ImportMapping`]).
    pub fn mapping_choices() -> MappingChoices {
        MappingChoices {
            delimiters: &DELIMITERS,
            date_formats: &DATE_FORMATS,
            amount_signs: &AMOUNT_SIGNS,
            decimal_separators: &DECIMAL_SEPARATORS,
        }
    }

    /// The company's bank accounts by name, active or not
    /// (`finance.reconciliation.view`).
    pub async fn list_bank_accounts(
        &self,
        scope: &Scope,
        company_id: i64,
    ) -> Result<Vec<BankAccount>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        Ok(bank_accounts::list(&**client, scope.tenant(), company_id).await?)
    }

    /// One bank account (`finance.reconciliation.view`).
    pub async fn get_bank_account(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
    ) -> Result<BankAccount> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        bank_accounts::find(&**client, scope.tenant(), company_id, bank_account_id)
            .await?
            .ok_or(ReconciliationError::BankAccountNotFound)
    }

    /// Adds a bank account (`finance.reconciliation.bank-accounts.manage`).
    /// Reads the chart through the ledger, so it also needs
    /// `finance.ledger.view`. Every field set is logged with no old value.
    pub async fn create_bank_account(
        &self,
        scope: &Scope,
        company_id: i64,
        new: &NewBankAccount,
    ) -> Result<BankAccount> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_BANK_ACCOUNTS],
                "finance.reconciliation.create_bank_account",
            )
            .await?;
        let chart = self
            .ledger
            .list_accounts_on(tx.sql(), scope, company_id)
            .await?
            .into_iter()
            .map(|account| (account.id, account))
            .collect();
        let valid = bank_accounts::validate_new(tx.sql(), scope.tenant(), company_id, &chart, new)
            .await?
            .map_err(ReconciliationError::Invalid)?;
        let tenant = scope.tenant();
        let (actor, impersonator) = people(scope);
        let account = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    bank_accounts::insert(sql, tenant, company_id, actor, impersonator, &valid)
                        .await
                })
            })
            .await?;
        tx.commit().await?;
        Ok(account)
    }

    /// Changes a bank account's name, counter accounts, match window, import
    /// mapping or active flag, with an optional reason
    /// (`finance.reconciliation.bank-accounts.manage`); its ledger account
    /// never changes. Every field that changes is logged
    /// ([`Reconciliation::list_bank_account_changes`]); a change that changes
    /// nothing records nothing.
    pub async fn update_bank_account(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
        change: &BankAccountEdit,
    ) -> Result<BankAccount> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[MANAGE_BANK_ACCOUNTS],
                "finance.reconciliation.update_bank_account",
            )
            .await?;
        let tenant = scope.tenant();
        let current = bank_accounts::find(tx.sql(), tenant, company_id, bank_account_id)
            .await?
            .ok_or(ReconciliationError::BankAccountNotFound)?;
        let chart = self
            .ledger
            .list_accounts_on(tx.sql(), scope, company_id)
            .await?
            .into_iter()
            .map(|account| (account.id, account))
            .collect();
        let Some(next) =
            bank_accounts::validate_change(tx.sql(), tenant, company_id, &chart, &current, change)
                .await?
                .map_err(ReconciliationError::Invalid)?
        else {
            return Ok(current);
        };
        let (actor, impersonator) = people(scope);
        let account = tx
            .mutate(move |sql| {
                Box::pin(async move {
                    bank_accounts::update(
                        sql,
                        tenant,
                        company_id,
                        actor,
                        impersonator,
                        &current,
                        &next,
                    )
                    .await
                })
            })
            .await?;
        tx.commit().await?;
        Ok(account)
    }

    /// The bank account's change log, oldest first
    /// (`finance.reconciliation.view`).
    pub async fn list_bank_account_changes(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
        page: &PageOptions,
    ) -> Result<Vec<BankAccountChange>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        bank_accounts::find(&**client, scope.tenant(), company_id, bank_account_id)
            .await?
            .ok_or(ReconciliationError::BankAccountNotFound)?;
        let (limit, after) = page.resolve()?;
        Ok(bank_accounts::log(
            &**client,
            scope.tenant(),
            company_id,
            bank_account_id,
            limit,
            after,
        )
        .await?)
    }

    /// Imports a bank statement (`finance.reconciliation.statements.import`):
    /// `file`, the CSV file's name and bytes (and its content type,
    /// `text/csv` unless given), is read through the bank account's import
    /// mapping into a numbered, immutable statement with its lines, which
    /// keeps that mapping as its `import_mapping`. The file itself is kept
    /// whole as a source document of the statement through
    /// `finance/attachments` ([`STATEMENT_SOURCE`]), retained as the company's
    /// retention says from the statement's end date, so the lines reproduce
    /// from it. The import also needs `finance.reconciliation.view` (the
    /// statement is read back when the file is linked to it),
    /// `finance.attachments.create` for the file and `finance.ledger.view` for
    /// the book's currency precision, all checked before anything is stored.
    ///
    /// The file is stored only once the statement passes every check below,
    /// before the transaction that records both (Base Artifacts stores a file
    /// outside any transaction), and a storage refusal imports nothing.
    /// Refused whole with `DuplicateFile` for a file already imported,
    /// `DuplicateLines` naming each line already imported (by this file's row
    /// and the statement and line that hold it), `InvalidLines` for rows the
    /// mapping cannot read, `MissingHeaders`, `NoLines`, `TooManyLines`,
    /// `RunningBalanceMismatch` when the balance column does not follow from
    /// the amounts in either direction, and the reader's `FileTooLarge`,
    /// `NotUtf8`, `StrayQuote` and `UnterminatedQuote`. A row whose amount is
    /// zero is skipped and counted in `skipped_count`.
    pub async fn import_statement(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
        file: &NewFile,
    ) -> Result<ImportedStatement> {
        let tenant = scope.tenant();
        {
            let client = self.db.get().await.map_err(Error::from)?;
            authorize(&**client, scope, company_id, IMPORT_STATEMENTS).await?;
            authorize(&**client, scope, company_id, VIEW).await?;
        }
        let account = self
            .active_bank_account(scope, company_id, bank_account_id)
            .await?;
        let book = self.ledger.get_book(scope, company_id).await?;
        let prepared = {
            let client = self.db.get().await.map_err(Error::from)?;
            import::prepare(
                &**client,
                tenant,
                company_id,
                &account,
                book.minor_units,
                file,
            )
            .await?
        };
        let capture = self
            .attachments
            .capture_file(
                scope,
                company_id,
                STATEMENT_SOURCE,
                &prepared.file,
                prepared.end_date,
            )
            .await?;
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company_id,
                &[IMPORT_STATEMENTS],
                "finance.reconciliation.import_statement",
            )
            .await?;
        // The bank account may have been deactivated, and another import may
        // have landed, since the checks above: ask again under the lock.
        bank_accounts::find(tx.sql(), tenant, company_id, account.id)
            .await?
            .filter(|account| account.active)
            .ok_or(ReconciliationError::BankAccountInactive)?;
        import::check_fresh(tx.sql(), tenant, company_id, account.id, &prepared).await?;
        let (actor, impersonator) = people(scope);
        let statement_id = import::insert(
            &mut tx,
            tenant,
            company_id,
            actor,
            impersonator,
            &account,
            &prepared,
        )
        .await?;
        self.attachments
            .attach_capture(
                &mut tx,
                scope,
                company_id,
                STATEMENT_SOURCE,
                statement_id,
                &capture,
            )
            .await?;
        let statement = statements::find(tx.sql(), tenant, company_id, statement_id, true)
            .await?
            .ok_or(Error::Internal)?;
        tx.commit().await?;
        Ok(ImportedStatement {
            statement,
            skipped_count: prepared.skipped,
        })
    }

    /// The bank account's statements, oldest first, without their lines
    /// (`finance.reconciliation.view`).
    pub async fn list_statements(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
        page: &PageOptions,
    ) -> Result<Vec<Statement>> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        bank_accounts::find(&**client, scope.tenant(), company_id, bank_account_id)
            .await?
            .ok_or(ReconciliationError::BankAccountNotFound)?;
        let (limit, after) = page.resolve()?;
        Ok(statements::list(
            &**client,
            scope.tenant(),
            company_id,
            bank_account_id,
            limit,
            after,
        )
        .await?)
    }

    /// One statement with its lines in line order
    /// (`finance.reconciliation.view`).
    pub async fn get_statement(
        &self,
        scope: &Scope,
        company_id: i64,
        statement_id: i64,
    ) -> Result<Statement> {
        let client = self.db.get().await.map_err(Error::from)?;
        authorize(&**client, scope, company_id, VIEW).await?;
        statements::find(&**client, scope.tenant(), company_id, statement_id, true)
            .await?
            .ok_or(ReconciliationError::StatementNotFound)
    }

    /// The bank account, which must be active.
    async fn active_bank_account(
        &self,
        scope: &Scope,
        company_id: i64,
        bank_account_id: i64,
    ) -> Result<BankAccount> {
        let client = self.db.get().await.map_err(Error::from)?;
        let account = bank_accounts::find(&**client, scope.tenant(), company_id, bank_account_id)
            .await?
            .ok_or(ReconciliationError::BankAccountNotFound)?;
        if account.active {
            Ok(account)
        } else {
            Err(ReconciliationError::BankAccountInactive)
        }
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
        Ok(self
            .ledger
            .write(client, scope, company_id, capabilities, action)
            .await?)
    }
}

/// The actor's current grant and company reach decide every call.
async fn authorize(
    client: &impl tokio_postgres::GenericClient,
    scope: &Scope,
    company_id: i64,
    capability: &str,
) -> Result<()> {
    organisation::authorize_target(client, scope, company_id, capability)
        .await
        .map(|_| ())
        .map_err(refusal)
}

fn refusal(error: Error) -> ReconciliationError {
    match error {
        Error::Forbidden | Error::Unauthorized => ReconciliationError::Unauthorized,
        Error::NotFound => ReconciliationError::CompanyNotFound,
        other => ReconciliationError::Failed(other),
    }
}

/// The scope's actor and any impersonator, as every record names them.
fn people(scope: &Scope) -> (i64, Option<i64>) {
    (
        scope.user(),
        scope.impersonator().map(|operator| operator.user()),
    )
}

fn as_text<T: std::fmt::Display, S: serde::Serializer>(
    value: &T,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.collect_str(value)
}
