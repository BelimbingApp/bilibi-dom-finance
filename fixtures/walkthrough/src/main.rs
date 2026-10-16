//! `DATABASE_URL=… cargo run -p bilibi-finance-walkthrough-fixture`, from the
//! Platform root with this container mounted, against a disposable database:
//! migrates it, installs the Platform and Finance walkthrough fixtures, and
//! then shows the ledger facade refusing and accepting what Bilimbi's does,
//! with the database's own refusals last. Every line is the facade's actual
//! answer; nothing here is a test double.
use bilibi_database::{pool, Error};
use bilibi_finance_ledger::{
    AccountChange, AccountType, Ledger, LedgerError, Money, NewAccount, NewBook, SERVICE,
};
use bilibi_finance_walkthrough_fixture::{install, sign_in, COMPANY};
use bilibi_module_registry::{verify_composition, Services};
use std::{fmt::Debug, sync::Arc};

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();

fn show<T: Debug>(step: &str, result: Result<T, LedgerError>) {
    match result {
        Ok(value) => println!("{step}: {value:?}"),
        Err(LedgerError::Invalid(errors)) => {
            let errors: Vec<String> = errors
                .iter()
                .map(|error| format!("{} {}", error.field, error.message))
                .collect();
            println!("{step}: refused ({})", errors.join("; "));
        }
        Err(error) => println!("{step}: refused ({error:?})"),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL").map_err(|_| Error::Invalid)?)?;
    let modules = bilibi_module_registry::discover()?;
    verify_composition(
        &modules,
        bilibi_domains::MODULES,
        bilibi_extensions::MODULES,
    )?;
    let migrated = bilibi_database::migrate(&db, &modules).await?;
    println!("mounted Domain modules: {:?}", bilibi_domains::MODULES);
    println!("migrations applied: {:?}", migrated.applied);
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.install(&modules)?;

    let installed = install(&db, &services).await?;
    println!("fixture book: {:?}", installed.book);
    for account in &installed.accounts {
        println!("fixture account: {account:?}");
    }
    let ledger = services.get::<Ledger>(SERVICE)?;
    let manager = sign_in(&db, &services, 1, 1).await?;
    let viewer = sign_in(&db, &services, 1, 2).await?;
    let admin = sign_in(&db, &services, 1, 4).await?;
    let other = sign_in(&db, &services, 2, 5).await?;
    let contractor = sign_in(&db, &services, 1, 7).await?;
    let find = |code: &str| {
        installed
            .accounts
            .iter()
            .find(|account| account.code == code)
            .map(|account| account.id)
            .ok_or(Error::NotFound)
    };
    let cash = find("1000")?;
    let receivables = find("1100")?;
    let sales = find("4000")?;
    let capital = find("3000")?;
    let retained = find("3100")?;

    println!("\n== Books");
    show(
        "manager reads company 1's book",
        ledger.get_book(&manager, COMPANY).await,
    );
    show(
        "manager opens company 1's book again",
        ledger
            .open_book(
                &manager,
                COMPANY,
                &NewBook {
                    currency_code: "MYR".into(),
                    minor_units: 2,
                },
            )
            .await,
    );
    show(
        "admin reads company 2's book",
        ledger.get_book(&admin, 2).await,
    );
    show(
        "admin opens company 2's book with a bad currency and 5 minor units",
        ledger
            .open_book(
                &admin,
                2,
                &NewBook {
                    currency_code: "ringgit".into(),
                    minor_units: 5,
                },
            )
            .await,
    );
    show(
        "contractor (staff, affiliated with company 2) opens company 2's book as ' usd ' with 2 minor units",
        ledger
            .open_book(
                &contractor,
                2,
                &NewBook {
                    currency_code: " usd ".into(),
                    minor_units: 2,
                },
            )
            .await,
    );

    println!("\n== Reach: grants, affiliation, tenancy, archival");
    show(
        "viewer (no affiliation, no tenant-wide reach) lists company 1's accounts",
        ledger.list_accounts(&viewer, COMPANY).await,
    );
    show(
        "contractor (affiliated with company 2, no tenant-wide reach) reads company 1's book",
        ledger.get_book(&contractor, COMPANY).await,
    );
    show(
        "manager (tenant-wide reach) reads company 2's book",
        ledger.get_book(&manager, 2).await,
    );
    show(
        "tenant 2's manager reads company 1's book",
        ledger.get_book(&other, COMPANY).await,
    );
    show(
        "tenant 2's manager reads tenant 2's company 100 book",
        ledger.get_book(&other, 100).await,
    );
    let client = db.get().await?;
    client
        .execute(
            "UPDATE companies SET status='archived' WHERE tenant_id=1 AND id=3",
            &[],
        )
        .await?;
    show(
        "admin opens archived company 3's book",
        ledger
            .open_book(
                &admin,
                3,
                &NewBook {
                    currency_code: "MYR".into(),
                    minor_units: 2,
                },
            )
            .await,
    );

    println!("\n== Chart of accounts");
    println!(
        "account types: {:?}",
        Ledger::account_types()
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
    );
    show(
        "manager adds 1000 again",
        ledger
            .create_account(
                &manager,
                COMPANY,
                &NewAccount::new("1000", "Petty cash", AccountType::Asset),
            )
            .await,
    );
    show(
        "manager adds a blank code and an over-long name",
        ledger
            .create_account(
                &manager,
                COMPANY,
                &NewAccount::new("  ", "n".repeat(201), AccountType::Asset),
            )
            .await,
    );
    show(
        "manager designates an income account for retained earnings",
        ledger
            .create_account(&manager, COMPANY, &{
                let mut new = NewAccount::new("4100", "Other income", AccountType::Income);
                new.retained_earnings = true;
                new
            })
            .await,
    );
    show(
        "manager groups an expense under the sales account",
        ledger
            .create_account(&manager, COMPANY, &{
                let mut new = NewAccount::new("5100", "Freight", AccountType::Expense);
                new.parent_id = Some(sales);
                new
            })
            .await,
    );
    show(
        "manager groups an account under another company's account id",
        ledger
            .create_account(&manager, COMPANY, &{
                let mut new = NewAccount::new("1200", "Deposits", AccountType::Asset);
                new.parent_id = Some(999_999);
                new
            })
            .await,
    );
    let petty = ledger
        .create_account(&manager, COMPANY, &{
            let mut new = NewAccount::new("1010", "Petty cash", AccountType::Asset);
            new.parent_id = Some(cash);
            new
        })
        .await?;
    println!("manager adds 1010 Petty cash under 1000: {petty:?}");
    let petty = petty.id;
    show(
        "manager renames 1010 and clears its parent",
        ledger
            .update_account(
                &manager,
                COMPANY,
                petty,
                &AccountChange {
                    name: Some("  Petty cash float  ".into()),
                    parent_id: Some(None),
                    ..AccountChange::default()
                },
            )
            .await,
    );
    show("manager places 1000 under 1010, which sits under 1000", {
        ledger
            .update_account(
                &manager,
                COMPANY,
                petty,
                &AccountChange {
                    parent_id: Some(Some(cash)),
                    ..AccountChange::default()
                },
            )
            .await?;
        ledger
            .update_account(
                &manager,
                COMPANY,
                cash,
                &AccountChange {
                    parent_id: Some(Some(petty)),
                    ..AccountChange::default()
                },
            )
            .await
    });
    show(
        "manager places 1000 under 1100 (a sibling: no cycle)",
        ledger
            .update_account(
                &manager,
                COMPANY,
                cash,
                &AccountChange {
                    parent_id: Some(Some(receivables)),
                    ..AccountChange::default()
                },
            )
            .await,
    );
    show(
        "manager deactivates 1010 (balance zero)",
        ledger
            .update_account(
                &manager,
                COMPANY,
                petty,
                &AccountChange {
                    active: Some(false),
                    ..AccountChange::default()
                },
            )
            .await,
    );
    show(
        "manager moves the retained-earnings designation from 3100 to 3000",
        ledger
            .update_account(
                &manager,
                COMPANY,
                capital,
                &AccountChange {
                    retained_earnings: Some(true),
                    ..AccountChange::default()
                },
            )
            .await,
    );
    show(
        "3100 after the move",
        ledger
            .list_accounts(&manager, COMPANY)
            .await
            .map(|accounts| accounts.into_iter().find(|a| a.id == retained)),
    );
    show(
        "manager designates the sales account",
        ledger
            .update_account(
                &manager,
                COMPANY,
                sales,
                &AccountChange {
                    retained_earnings: Some(true),
                    ..AccountChange::default()
                },
            )
            .await,
    );
    show(
        "manager updates an account id of another company",
        ledger
            .update_account(
                &manager,
                COMPANY,
                999_999,
                &AccountChange {
                    active: Some(false),
                    ..AccountChange::default()
                },
            )
            .await,
    );
    show(
        "admin (grant_all, tenant-wide) lists company 1's accounts",
        ledger
            .list_accounts(&admin, COMPANY)
            .await
            .map(|accounts| accounts.len()),
    );

    println!("\n== Money in the book's minor units");
    let units = installed.book.minor_units;
    for text in ["12.50", "12.505", "-3", "10000000000000000", "twelve"] {
        match Money::parse(text, units) {
            Ok(money) => println!("{text:>8}: {} (shown {})", money, money.shown(units)),
            Err(error) => println!("{text:>8}: refused ({error})"),
        }
    }

    println!("\n== The database's own refusals");
    for (step, sql) in [
        (
            "UPDATE a book's currency",
            "UPDATE finance_ledger_books SET currency_code='USD' WHERE company_id=1",
        ),
        (
            "DELETE a book",
            "DELETE FROM finance_ledger_books WHERE company_id=1",
        ),
        (
            "DELETE an account",
            "DELETE FROM finance_ledger_accounts WHERE company_id=1 AND code='1000'",
        ),
        (
            "UPDATE an account's code",
            "UPDATE finance_ledger_accounts SET code='9999' WHERE company_id=1 AND code='1000'",
        ),
        (
            "UPDATE an account's type",
            "UPDATE finance_ledger_accounts SET account_type='equity' WHERE company_id=1 AND code='1000'",
        ),
        (
            "INSERT a second retained-earnings account",
            "UPDATE finance_ledger_accounts SET retained_earnings=true WHERE company_id=1 AND code='3100'",
        ),
        (
            "INSERT an account with a parent of another company",
            "INSERT INTO finance_ledger_accounts(tenant_id,company_id,code,name,account_type,parent_id,inserted_at,updated_at) \
             SELECT 1,2,'1000','Cash','asset',id,now(),now() FROM finance_ledger_accounts WHERE company_id=1 AND code='1000'",
        ),
        (
            "INSERT a bookkeeping change that changes nothing",
            "INSERT INTO finance_ledger_bookkeeping_changes(tenant_id,company_id,change_number,controlled,changed_by_user_id,changed_at) VALUES(1,1,NULL,true,1,NULL)",
        ),
        (
            "TRUNCATE the books",
            "TRUNCATE finance_ledger_books",
        ),
    ] {
        match client.batch_execute(sql).await {
            Ok(()) => println!("{step}: accepted (!)"),
            Err(error) => println!(
                "{step}: refused ({})",
                error
                    .as_db_error()
                    .map(|e| e.message().to_owned())
                    .unwrap_or_else(|| error.to_string())
            ),
        }
    }
    let changes: Vec<(i64, bool)> = client
        .query(
            "SELECT change_number,controlled FROM finance_ledger_bookkeeping_changes WHERE company_id=1 ORDER BY change_number",
            &[],
        )
        .await?
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    println!("company 1's bookkeeping changes: {changes:?}");
    let audited: i64 = client
        .query_one(
            "SELECT count(*) FROM base_audit_mutations WHERE auditable_type LIKE 'finance.ledger.%'",
            &[],
        )
        .await?
        .get(0);
    println!("audit mutations recorded for finance.ledger.*: {audited}");
    Ok(())
}
