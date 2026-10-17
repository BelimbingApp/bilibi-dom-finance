//! The Finance walkthrough fixture: the ledger records the slice proofs act
//! on, created through the facade on top of the Platform's walkthrough
//! fixture (its tenants, accounts, roles and companies). It is not product
//! data: only examples depend on this crate, and only a disposable, freshly
//! migrated database may receive it (the Platform fixture refuses any other).
//! `install` is idempotent: a rerun finds the book open and the chart in
//! place and creates nothing.
//!
//! Company 001 (tenant 1's primary company) gets a book in [`CURRENCY`] with
//! [`MINOR_UNITS`] decimal places and the chart in [`CHART`], created by the
//! fixture's `manager@example.test`, who is affiliated with that company, and
//! its first fiscal year, [`FISCAL_YEAR_START`] for twelve months, since a new
//! book is controlled and posts only into a period it has opened. The
//! fixture's roles hold ledger capabilities only through
//! [`walkthrough_ledger_roles!`], which the expanding example declares beside
//! the Platform fixture's two macros: the manager holds every ledger
//! capability, staff view and maintain the chart, the viewer only views.
use bilibi_authz::grants::Grants;
use bilibi_database::{Db, Error};
use bilibi_finance_ledger::{
    Account, AccountType, Book, Ledger, LedgerError, NewAccount, NewBook, NewFiscalYear, Result,
    SERVICE,
};
use bilibi_module_registry::Services;
use bilibi_tenancy::{Authentication, Scope};
use jiff::civil::{date, Date};

pub mod attachments;
pub mod controls;
pub mod payables;
pub mod periods;

/// Every fixture account's password.
pub use bilibi_walkthrough_fixture::PASSWORD;
#[doc(hidden)]
pub use {bilibi_authz as authz, bilibi_module_registry as registry, inventory};

/// The company whose book the fixture opens: Company 001, tenant 1's primary.
pub const COMPANY: i64 = 1;
/// The fixture manager's account (`manager@example.test`), affiliated with
/// Company 001.
pub const MANAGER: i64 = 1;
pub const CURRENCY: &str = "MYR";
/// The first day of Company 001's first fiscal year.
pub const FISCAL_YEAR_START: Date = date(2026, 1, 1);
pub const MINOR_UNITS: i64 = 2;
/// Code, name, type and whether the account is the retained-earnings one.
pub const CHART: [(&str, &str, AccountType, bool); 9] = [
    ("1000", "Cash at bank", AccountType::Asset, false),
    ("1100", "Trade receivables", AccountType::Asset, false),
    ("2000", "Trade payables", AccountType::Liability, false),
    ("2100", "Tax payable", AccountType::Liability, false),
    ("3000", "Share capital", AccountType::Equity, false),
    ("3100", "Retained earnings", AccountType::Equity, true),
    ("4000", "Sales", AccountType::Income, false),
    ("5000", "Cost of sales", AccountType::Expense, false),
    ("6000", "Bank charges", AccountType::Expense, false),
];

/// What the fixture left in place.
#[derive(Debug)]
pub struct Installed {
    pub book: Book,
    pub accounts: Vec<Account>,
}

/// Installs the Platform fixture, reconciles the contributed roles, and opens
/// Company 001's book and chart through the ledger facade as the manager.
pub async fn install(db: &Db, services: &Services) -> Result<Installed> {
    bilibi_walkthrough_fixture::install(db).await?;
    services
        .get::<Grants>("base/authz.grants")
        .map_err(Error::from)?
        .reconcile()
        .await?;
    let ledger = services.get::<Ledger>(SERVICE).map_err(Error::from)?;
    let manager = sign_in(db, services, 1, MANAGER).await?;
    let book = match ledger.get_book(&manager, COMPANY).await {
        Err(LedgerError::BookNotOpen) => {
            ledger
                .open_book(
                    &manager,
                    COMPANY,
                    &NewBook {
                        currency_code: CURRENCY.into(),
                        minor_units: MINOR_UNITS,
                    },
                )
                .await?
        }
        other => other?,
    };
    let existing = ledger.list_accounts(&manager, COMPANY).await?;
    for (code, name, account_type, retained_earnings) in CHART {
        if existing.iter().any(|account| account.code == code) {
            continue;
        }
        let mut new = NewAccount::new(code, name, account_type);
        new.retained_earnings = retained_earnings;
        ledger.create_account(&manager, COMPANY, &new).await?;
    }
    let accounts = ledger.list_accounts(&manager, COMPANY).await?;
    if ledger
        .list_fiscal_years(&manager, COMPANY)
        .await?
        .is_empty()
    {
        ledger
            .open_fiscal_year(
                &manager,
                COMPANY,
                &NewFiscalYear {
                    start_date: Some(FISCAL_YEAR_START),
                    end_date: None,
                },
            )
            .await?;
    }
    records::install(db).await?;
    Ok(Installed { book, accounts })
}

/// A sealed scope for one fixture account, as a signed-in session would hold.
pub async fn sign_in(db: &Db, services: &Services, tenant: i64, user: i64) -> Result<Scope> {
    let authentication = services
        .get::<Authentication>("base/tenancy.authentication")
        .map_err(Error::from)?;
    let client = db.get().await.map_err(Error::from)?;
    Ok(authentication
        .authenticate(&**client, tenant, user, String::new())
        .await?)
}

/// What the Platform fixture's roles hold in the ledger, declared in the
/// expanding example's own crate beside
/// `bilibi_walkthrough_fixture::walkthrough_vocabulary!` and
/// `walkthrough_module_roles!`.
#[macro_export]
macro_rules! walkthrough_ledger_roles {
    () => {
        $crate::inventory::submit! { $crate::registry::Contribution::new($crate::authz::VOCABULARY, "walkthrough.fixture.finance.ledger", $crate::registry::Provenance { module: "finance/ledger", source: file!(), line: line!() }, &$crate::authz::VocabularyContribution {
            domains: &[],
            verbs: &[],
            capabilities: &[],
            roles: &[
                $crate::authz::RoleDefinition { code: "manager", name: "manager", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.ledger.view", "finance.ledger.accounts.manage", "finance.ledger.entries.post",
                    "finance.ledger.periods.manage", "finance.ledger.periods.reopen", "finance.ledger.periods.lock",
                    "finance.ledger.duty-separation.manage", "finance.ledger.controlled-bookkeeping.manage",
                    "finance.ledger.late-recordings.review", "finance.ledger.journals.submit",
                    "finance.ledger.journals.approve", "finance.ledger.approval-exceptions.review",
                ] },
                $crate::authz::RoleDefinition { code: "staff", name: "staff", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.ledger.view", "finance.ledger.accounts.manage",
                ] },
                $crate::authz::RoleDefinition { code: "viewer", name: "viewer", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.ledger.view",
                ] },
            ],
            system_principals: &[],
        }) }
    };
}

/// The walkthrough's records: what the specimen sources of source documents
/// describe, in a table only a disposable database holds. A real Finance
/// module owns its records and contributes its own `Source`; this table
/// stands in for them so the attachments facade can be shown on a draft and
/// a final record.
pub mod records {
    use bilibi_company::organisation;
    use bilibi_database::{Db, Error};
    use bilibi_finance_attachments::{
        on_source_client, Source, SourceClient, SourceDefinition, SourceDocument, SourceFuture,
        SourceRefusal, SOURCES,
    };
    use bilibi_module_registry::{Contribution, Provenance};
    use bilibi_tenancy::Scope;
    use jiff::civil::Date;
    use tokio_postgres::GenericClient;

    /// A record any ledger reader reads.
    pub const OPEN_SOURCE: &str = "finance/attachments/specimen_record";
    /// A record only a poster of entries reads.
    pub const RESTRICTED_SOURCE: &str = "finance/attachments/restricted_record";

    struct Records {
        kind: &'static str,
        label: &'static str,
        view: &'static str,
    }

    impl Source for Records {
        fn label(&self) -> &'static str {
            self.label
        }
        fn view_capability(&self) -> &'static str {
            self.view
        }
        fn document<'a>(
            &'a self,
            client: SourceClient<'a>,
            scope: &'a Scope,
            company_id: i64,
            document_id: i64,
        ) -> SourceFuture<'a> {
            Box::pin(async move {
                on_source_client!(client, |c| self
                    .read(c, scope, company_id, document_id)
                    .await)
            })
        }
    }

    impl Records {
        async fn read(
            &self,
            client: &(impl GenericClient + Sync),
            scope: &Scope,
            company_id: i64,
            document_id: i64,
        ) -> Result<SourceDocument, SourceRefusal> {
            organisation::authorize_target(client, scope, company_id, self.view)
                .await
                .map_err(|error| match error {
                    Error::Forbidden | Error::Unauthorized => SourceRefusal::Unauthorized,
                    Error::NotFound => SourceRefusal::NotFound,
                    other => SourceRefusal::Failed(other),
                })?;
            let row = client
                .query_opt(
                    "SELECT label,to_char(record_date,'YYYY-MM-DD'),is_final FROM finance_walkthrough_records \
                     WHERE tenant_id=$1 AND company_id=$2 AND id=$3 AND kind=$4",
                    &[&scope.tenant(), &company_id, &document_id, &self.kind],
                )
                .await
                .map_err(Error::from)?
                .ok_or(SourceRefusal::NotFound)?;
            Ok(SourceDocument {
                label: row.get(0),
                date: row.get::<_, &str>(1).parse().map_err(|_| Error::Internal)?,
                is_final: row.get(2),
            })
        }
    }

    static OPEN: Records = Records {
        kind: "open",
        label: "Specimen record",
        view: "finance.ledger.view",
    };
    static RESTRICTED: Records = Records {
        kind: "restricted",
        label: "Restricted record",
        view: "finance.ledger.entries.post",
    };
    // The specimen sources are contributed as the attachments module, the
    // only registered module this example-only crate can speak for.
    inventory::submit! { Contribution::new(SOURCES, OPEN_SOURCE,
    Provenance { module: "finance/attachments", source: file!(), line: line!() },
    &SourceDefinition { source: &OPEN }) }
    inventory::submit! { Contribution::new(SOURCES, RESTRICTED_SOURCE,
    Provenance { module: "finance/attachments", source: file!(), line: line!() },
    &SourceDefinition { source: &RESTRICTED }) }

    /// Creates the table in a disposable database; a rerun finds it in place.
    pub async fn install(db: &Db) -> Result<(), Error> {
        let client = db.get().await?;
        client
            .batch_execute(
                "CREATE TABLE IF NOT EXISTS finance_walkthrough_records (\
                   id bigserial PRIMARY KEY, tenant_id bigint NOT NULL, company_id bigint NOT NULL, \
                   kind text NOT NULL, label text NOT NULL, record_date date NOT NULL, \
                   is_final boolean NOT NULL DEFAULT false)",
            )
            .await
            .map_err(Error::from)
    }

    /// Adds a record of `kind` (`open` or `restricted`) and returns its id.
    pub async fn add(
        db: &Db,
        tenant: i64,
        company_id: i64,
        kind: &str,
        label: &str,
        date: Date,
        is_final: bool,
    ) -> Result<i64, Error> {
        let client = db.get().await?;
        Ok(client
            .query_one(
                "INSERT INTO finance_walkthrough_records(tenant_id,company_id,kind,label,record_date,is_final) \
                 VALUES($1,$2,$3,$4,$5::text::date,$6) RETURNING id",
                &[&tenant, &company_id, &kind, &label, &date.to_string(), &is_final],
            )
            .await
            .map_err(Error::from)?
            .get(0))
    }

    /// Re-dates a record and/or finalises it, as its owning module would.
    pub async fn set(db: &Db, id: i64, date: Date, is_final: bool) -> Result<(), Error> {
        let client = db.get().await?;
        client
            .execute(
                "UPDATE finance_walkthrough_records SET record_date=$2::text::date, is_final=$3 WHERE id=$1",
                &[&id, &date.to_string(), &is_final],
            )
            .await
            .map_err(Error::from)?;
        Ok(())
    }
}

/// What the Platform fixture's roles hold in the attachments module: the
/// manager every capability, staff view and attach, the viewer only view.
/// Declared in the expanding example's own crate beside
/// [`walkthrough_ledger_roles!`].
#[macro_export]
macro_rules! walkthrough_attachments_roles {
    () => {
        $crate::inventory::submit! { $crate::registry::Contribution::new($crate::authz::VOCABULARY, "walkthrough.fixture.finance.attachments", $crate::registry::Provenance { module: "finance/attachments", source: file!(), line: line!() }, &$crate::authz::VocabularyContribution {
            domains: &[],
            verbs: &[],
            capabilities: &[],
            roles: &[
                $crate::authz::RoleDefinition { code: "manager", name: "manager", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.attachments.view", "finance.attachments.create", "finance.attachments.delete",
                    "finance.attachments.retention.manage",
                ] },
                $crate::authz::RoleDefinition { code: "staff", name: "staff", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.attachments.view", "finance.attachments.create",
                ] },
                $crate::authz::RoleDefinition { code: "viewer", name: "viewer", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.attachments.view",
                ] },
            ],
            system_principals: &[],
        }) }
    };
}
