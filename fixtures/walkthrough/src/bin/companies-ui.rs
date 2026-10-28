//! Browser fixture for the pages that act on the session's current company:
//! the Finance setup page, Company books, and the Fiscal years company filter.
//! Run with DATABASE_URL, APP_ORIGIN, BIND_ADDR and BILIBI_SETTINGS_KEY against
//! a disposable database. Identity fixture SQL only; every Finance record is
//! created by its facade.
//!
//! Tenant 1's companies (ids 1 to 62) hold four states: Company 001 has an
//! open book with a chart, the install's first fiscal year and payables set up; Company 002 has an open JPY book (no
//! decimal places) whose first fiscal year is a short six months; Company 003
//! has an open USD book and no fiscal year; every other company has no book.
//!
//! Sign in, with the walkthrough password, as `manager@example.test` (works
//! in every company of the tenant, affiliated with Company 001),
//! `contractor@example.test` (works only in Company 002) or
//! `viewer@example.test` (no company at all).
use bilibi_database::pool;
use bilibi_finance_ledger::{Ledger, NewAccount, NewBook, NewFiscalYear};
use bilibi_finance_payables::{Payables, Settings, SettingsInput};
use bilibi_finance_walkthrough_fixture::{install, sign_in};
use jiff::civil::date;

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_payables_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_receivables_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_reports_roles!();
// The company-only accountant also manages periods, to show one company's
// fiscal years being opened and closed without reach to another.
inventory::submit! { bilibi_module_registry::Contribution::new(bilibi_authz::VOCABULARY, "walkthrough.fixture.finance.accountant", bilibi_module_registry::Provenance { module: "finance/ledger", source: file!(), line: line!() }, &bilibi_authz::VocabularyContribution {
    domains: &[], verbs: &[], capabilities: &[], system_principals: &[],
    roles: &[bilibi_authz::RoleDefinition { code: "staff", name: "staff", description: "Walkthrough fixture role", grant_all: false, capabilities: &["finance.ledger.periods.manage", "finance.ledger.entries.post"] }],
}) }

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> Outcome<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let origin = std::env::var("APP_ORIGIN")?;
    bilibi_database::migrate(&db, &bilibi_module_registry::discover()?).await?;
    let app = bilibi_web::App::new(db.clone(), &origin)?;
    bilibi_finance_walkthrough_fixture::without_step_up(&app.services)?;
    let services = &app.services;
    install(&db, services).await?;
    let manager = sign_in(&db, services, 1, 1).await?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let payables = services.get::<Payables>(bilibi_finance_payables::SERVICE)?;
    let year = jiff::Zoned::now().year();

    // `install` opened Company 001's book, chart and first fiscal year.
    if matches!(
        ledger.get_book(&manager, 2).await,
        Err(bilibi_finance_ledger::LedgerError::BookNotOpen)
    ) {
        for (company, currency, units) in [(2, "JPY", 0), (3, "USD", 2)] {
            ledger
                .open_book(
                    &manager,
                    company,
                    &NewBook {
                        currency_code: currency.into(),
                        minor_units: units,
                    },
                )
                .await?;
        }
        // Company 002 gets the walkthrough chart; Company 003 stays bare.
        for (code, name, kind, retained) in bilibi_finance_walkthrough_fixture::CHART {
            let mut account = NewAccount::new(code, name, kind);
            account.retained_earnings = retained;
            ledger.create_account(&manager, 2, &account).await?;
        }
        // Company 002: a short first year of six months.
        ledger
            .open_fiscal_year(
                &manager,
                2,
                &NewFiscalYear {
                    start_date: Some(date(year - 1, 4, 1)),
                    end_date: Some(date(year - 1, 9, 30)),
                },
            )
            .await?;
        let payable = ledger
            .list_accounts(&manager, 1)
            .await?
            .into_iter()
            .find(|account| account.code == "2000")
            .map(|account| account.id);
        if let Some(account) = payable {
            payables
                .put_settings(
                    &manager,
                    1,
                    &SettingsInput {
                        settings: Settings {
                            payable_account_id: account,
                            quantity_tolerance: None,
                            price_tolerance: None,
                        },
                        reason: None,
                    },
                )
                .await?;
        }
    }
    println!(
        "Current-company fixture at {origin}: manager@example.test (every company), contractor@example.test (Company 002 only), viewer@example.test (no company); password bilibi-dev"
    );
    axum::serve(
        tokio::net::TcpListener::bind(std::env::var("BIND_ADDR")?).await?,
        bilibi_web::router(app),
    )
    .await?;
    Ok(())
}
