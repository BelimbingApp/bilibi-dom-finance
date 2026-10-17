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
//! fixture's `manager@example.test`, who is affiliated with that company. The
//! fixture's roles hold ledger capabilities only through
//! [`walkthrough_ledger_roles!`], which the expanding example declares beside
//! the Platform fixture's two macros: the manager holds every ledger
//! capability, staff view and maintain the chart, the viewer only views.
use bilibi_authz::grants::Grants;
use bilibi_database::{Db, Error};
use bilibi_finance_ledger::{
    Account, AccountType, Book, Ledger, LedgerError, NewAccount, NewBook, Result, SERVICE,
};
use bilibi_module_registry::Services;
use bilibi_tenancy::{Authentication, Scope};

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

/// What the Platform fixture's roles hold in e-invoicing, declared beside
/// [`walkthrough_ledger_roles!`]: the manager every capability, staff and the
/// viewer only the read.
#[macro_export]
macro_rules! walkthrough_einvoice_roles {
    () => {
        $crate::inventory::submit! { $crate::registry::Contribution::new($crate::authz::VOCABULARY, "walkthrough.fixture.finance.einvoice", $crate::registry::Provenance { module: "finance/einvoice", source: file!(), line: line!() }, &$crate::authz::VocabularyContribution {
            domains: &[],
            verbs: &[],
            capabilities: &[],
            roles: &[
                $crate::authz::RoleDefinition { code: "manager", name: "manager", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.einvoice.view", "finance.einvoice.documents.issue", "finance.einvoice.documents.submit",
                    "finance.einvoice.documents.cancel", "finance.einvoice.settings.manage",
                ] },
                $crate::authz::RoleDefinition { code: "staff", name: "staff", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.einvoice.view",
                ] },
                $crate::authz::RoleDefinition { code: "viewer", name: "viewer", description: "Walkthrough fixture role", grant_all: false, capabilities: &[
                    "finance.einvoice.view",
                ] },
            ],
            system_principals: &[],
        }) }
    };
}
