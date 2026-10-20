//! Browser fixture for the ledger pages (books, chart of accounts, journal
//! entries, manual journals, fiscal years, trial balance). Run with
//! DATABASE_URL, APP_ORIGIN, BIND_ADDR and BILIBI_SETTINGS_KEY against a
//! disposable database. Identity fixture SQL only; every ledger record is
//! created by the ledger's facade, as the pages and agents create them.
//!
//! Sign in as `manager@other.example.test` (every ledger capability, in Company
//! 100), `reviewer@other.example.test` (the administrator, who approves the
//! manager's journals) or `viewer@other.example.test` (view only), with the
//! walkthrough password. Pass `empty` to stop before the book is opened.
use bilibi_database::pool;
use bilibi_finance_ledger::{
    AccountType, Ledger, Money, NewAccount, NewBook, NewEntry, NewFiscalYear, NewLine, PostOptions,
};
use bilibi_finance_walkthrough_fixture::{install, sign_in};
use jiff::civil::{date, Date};

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();

const COMPANY: i64 = 100;

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
    db.get().await?.batch_execute(
        "INSERT INTO persons(id,tenant_id,name,kind,email) VALUES(4,1,'Alex Admin','human','admin@example.test'),(5,1,'Lee Viewer','human','viewer@example.test') ON CONFLICT DO NOTHING;
         INSERT INTO person_affiliations(id,tenant_id,person_id,organisation_id,title,work_email) VALUES(3,1,4,1,'Reviewer','admin@example.test'),(4,1,5,1,'Reader','viewer@example.test') ON CONFLICT DO NOTHING;
         UPDATE users SET person_id=4,affiliation_id=3,organisation_id=1 WHERE tenant_id=1 AND id=4 AND person_id IS NULL;
         UPDATE users SET person_id=5,affiliation_id=4,organisation_id=1 WHERE tenant_id=1 AND id=2 AND person_id IS NULL;
         SELECT setval(pg_get_serial_sequence('persons','id'),(SELECT max(id) FROM persons));
         SELECT setval(pg_get_serial_sequence('person_affiliations','id'),(SELECT max(id) FROM person_affiliations))"
    ).await?;
    // Tenant 2 is an ordinary tenant: a manager, an administrator who reviews,
    // and a reader. None of them is a platform operator.
    db.get().await?.batch_execute(
        "INSERT INTO persons(id,tenant_id,name,kind,email) VALUES(6,2,'Other Manager','human','manager@other.example.test'),(7,2,'Other Reviewer','human','reviewer@other.example.test'),(8,2,'Other Reader','human','viewer@other.example.test') ON CONFLICT DO NOTHING;
         INSERT INTO person_affiliations(id,tenant_id,person_id,organisation_id,title) VALUES(5,2,6,100,'Recorder'),(6,2,7,100,'Reviewer'),(7,2,8,100,'Reader') ON CONFLICT DO NOTHING;
         UPDATE users SET person_id=6,affiliation_id=5,organisation_id=100 WHERE tenant_id=2 AND id=5 AND person_id IS NULL;
         INSERT INTO users(id,tenant_id,name,email,password_hash,person_id,affiliation_id,organisation_id) SELECT 8,2,'Other Reviewer','reviewer@other.example.test',password_hash,7,6,100 FROM users WHERE id=5 ON CONFLICT DO NOTHING;
         INSERT INTO users(id,tenant_id,name,email,password_hash,person_id,affiliation_id,organisation_id) SELECT 9,2,'Other Reader','viewer@other.example.test',password_hash,8,7,100 FROM users WHERE id=5 ON CONFLICT DO NOTHING;
         INSERT INTO principal_roles(tenant_id,user_id,role_id) SELECT 2,8,id FROM roles WHERE tenant_id=2 AND code='admin' ON CONFLICT DO NOTHING;
         INSERT INTO principal_roles(tenant_id,user_id,role_id) SELECT 2,9,id FROM roles WHERE tenant_id=2 AND code='viewer' ON CONFLICT DO NOTHING;
         SELECT setval(pg_get_serial_sequence('persons','id'),(SELECT max(id) FROM persons));
         SELECT setval(pg_get_serial_sequence('person_affiliations','id'),(SELECT max(id) FROM person_affiliations));
         SELECT setval(pg_get_serial_sequence('users','id'),(SELECT max(id) FROM users))"
    ).await?;
    let manager = sign_in(&db, services, 2, 5).await?;
    let reviewer = sign_in(&db, services, 2, 8).await?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;

    let unopened = matches!(
        ledger.get_book(&manager, COMPANY).await,
        Err(bilibi_finance_ledger::LedgerError::BookNotOpen)
    );
    if unopened && !std::env::args().any(|arg| arg == "empty") {
        seed(&ledger, &manager, &reviewer).await?;
    }
    println!(
        "Ledger fixture at {origin}: manager@other.example.test, reviewer@other.example.test, viewer@other.example.test; walkthrough password"
    );
    axum::serve(
        tokio::net::TcpListener::bind(std::env::var("BIND_ADDR")?).await?,
        bilibi_web::router(app),
    )
    .await?;
    Ok(())
}

/// A small company's two years: a chart, entries through the months, closed
/// periods, a closed year, and manual journals in each state.
async fn seed(
    ledger: &Ledger,
    manager: &bilibi_tenancy::Scope,
    reviewer: &bilibi_tenancy::Scope,
) -> Outcome<()> {
    let book = ledger
        .open_book(
            manager,
            COMPANY,
            &NewBook {
                currency_code: "MYR".into(),
                minor_units: 2,
            },
        )
        .await?;
    let units = book.minor_units;
    let mut ids = std::collections::HashMap::new();
    let chart: [(&str, &str, AccountType, bool); 14] = [
        ("1000", "Cash at bank", AccountType::Asset, false),
        ("1100", "Trade receivables", AccountType::Asset, false),
        ("1200", "Equipment", AccountType::Asset, false),
        ("2000", "Trade payables", AccountType::Liability, false),
        ("2100", "Tax payable", AccountType::Liability, false),
        ("2200", "Accrued expenses", AccountType::Liability, false),
        ("3000", "Share capital", AccountType::Equity, false),
        ("3100", "Retained earnings", AccountType::Equity, true),
        ("4000", "Sales", AccountType::Income, false),
        ("4100", "Service income", AccountType::Income, false),
        ("5000", "Cost of sales", AccountType::Expense, false),
        ("5100", "Rent", AccountType::Expense, false),
        ("5200", "Salaries", AccountType::Expense, false),
        ("6000", "Bank charges", AccountType::Expense, false),
    ];
    for (code, name, kind, retained) in chart {
        let mut account = NewAccount::new(code, name, kind);
        account.retained_earnings = retained;
        ids.insert(
            code,
            ledger.create_account(manager, COMPANY, &account).await?.id,
        );
    }
    // A group under cash: the operating account.
    let mut operating = NewAccount::new("1010", "Operating account", AccountType::Asset);
    operating.parent_id = Some(ids["1000"]);
    ids.insert(
        "1010",
        ledger
            .create_account(manager, COMPANY, &operating)
            .await?
            .id,
    );

    let this_year = jiff::Zoned::now().year();
    let first = date(this_year - 1, 1, 1);
    ledger
        .open_fiscal_year(
            manager,
            COMPANY,
            &NewFiscalYear {
                start_date: Some(first),
                end_date: None,
            },
        )
        .await?;
    ledger
        .open_fiscal_year(manager, COMPANY, &NewFiscalYear::default())
        .await?;

    let money = |text: &str| Money::parse(text, units);
    let post = |day: Date,
                description: &str,
                reference: Option<&str>,
                debit: &str,
                credit: &str,
                amount: &str| {
        let entry = NewEntry {
            entry_date: day,
            description: description.into(),
            reference: reference.map(str::to_owned),
            lines: vec![
                NewLine::debit(ids[debit], money(amount).expect("fixture amount")),
                NewLine::credit(ids[credit], money(amount).expect("fixture amount")),
            ],
        };
        async move {
            ledger
                .post_entry(manager, COMPANY, &entry, &PostOptions::default())
                .await
        }
    };
    let last = this_year - 1;
    post(
        date(last, 1, 2),
        "Shareholders' capital paid in",
        Some("CAP-001"),
        "1010",
        "3000",
        "50000.00",
    )
    .await?;
    post(
        date(last, 2, 10),
        "Equipment bought",
        Some("INV-7781"),
        "1200",
        "1010",
        "12000.00",
    )
    .await?;
    for month in 3..=11 {
        post(
            date(last, month, 15),
            "Sales for the month",
            None,
            "1010",
            "4000",
            "9500.00",
        )
        .await?;
        post(
            date(last, month, 20),
            "Rent for the month",
            Some("RENT"),
            "5100",
            "1010",
            "2500.00",
        )
        .await?;
        post(
            date(last, month, 28),
            "Salaries for the month",
            None,
            "5200",
            "1010",
            "4200.00",
        )
        .await?;
    }
    post(
        date(last, 12, 5),
        "Service income, consulting",
        Some("SVC-12"),
        "1100",
        "4100",
        "3800.00",
    )
    .await?;
    post(
        date(last, 12, 31),
        "Bank charges for the year",
        None,
        "6000",
        "1010",
        "96.40",
    )
    .await?;
    for month in 1..=9 {
        post(
            date(this_year, month, 14),
            "Sales for the month",
            None,
            "1010",
            "4000",
            "11200.50",
        )
        .await?;
        post(
            date(this_year, month, 20),
            "Rent for the month",
            Some("RENT"),
            "5100",
            "1010",
            "2500.00",
        )
        .await?;
        post(
            date(this_year, month, 27),
            "Salaries for the month",
            None,
            "5200",
            "1010",
            "4350.00",
        )
        .await?;
    }
    let wrong = post(
        date(this_year, 9, 30),
        "Office supplies, wrong account",
        Some("EXP-221"),
        "6000",
        "1010",
        "318.75",
    )
    .await?;

    // Close last year, then the first eight periods of this one.
    let years = ledger.list_fiscal_years(manager, COMPANY).await?;
    ledger
        .close_fiscal_year(manager, COMPANY, years[0].id, &Default::default())
        .await?;
    for period in years[1].periods.iter().take(8) {
        ledger
            .close_period(manager, COMPANY, period.id, None)
            .await?;
    }

    // Manual journals: one waiting, one approved, one rejected.
    let adjustment =
        |day: Date, description: &str, debit: &str, credit: &str, amount: &str| NewEntry {
            entry_date: day,
            description: description.into(),
            reference: Some("ADJ".into()),
            lines: vec![
                NewLine::debit(ids[debit], money(amount).expect("fixture amount"))
                    .described("Adjustment"),
                NewLine::credit(ids[credit], money(amount).expect("fixture amount")),
            ],
        };
    let approved = ledger
        .submit_journal(
            manager,
            COMPANY,
            &adjustment(
                date(this_year, 9, 30),
                "Accrue September audit fee",
                "5100",
                "2200",
                "1800.00",
            ),
        )
        .await?;
    ledger
        .approve_journal(reviewer, COMPANY, approved.id)
        .await?;
    let rejected = ledger
        .submit_journal(
            manager,
            COMPANY,
            &adjustment(
                date(this_year, 9, 30),
                "Write off a receivable",
                "6000",
                "1100",
                "950.00",
            ),
        )
        .await?;
    ledger
        .reject_journal(
            reviewer,
            COMPANY,
            rejected.id,
            "The customer has paid. No write-off.",
        )
        .await?;
    ledger
        .submit_journal(
            manager,
            COMPANY,
            &adjustment(
                date(this_year, 10, 15),
                "Accrue October salaries",
                "5200",
                "2200",
                "4350.00",
            ),
        )
        .await?;
    println!(
        "Ledger seeded: wrong-account entry {}; journal waiting for approval is the latest",
        wrong.id
    );
    Ok(())
}
