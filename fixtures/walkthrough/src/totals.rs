//! The totals walkthrough: trial balances, account totals (and by owner) at a
//! pinned entry, and the ledger's evidence pages, on Company 001 and Company
//! 002 after every other walkthrough has posted, so the figures cover manual,
//! owned, reversed, journal and year-end closing entries. Every line is the
//! facade's actual answer; nothing here is a test double.
use crate::sign_in;
use bilibi_database::Db;
use bilibi_finance_ledger::{
    AccountTotals, EntryQuery, Ledger, LedgerError, Money, NewEntry, NewLine, OwnerTotals,
    PeriodStatus, PostOptions, TotalsQuery, TrialBalance, YearStatus, SERVICE,
};
use bilibi_module_registry::Services;
use jiff::civil::date;
use std::fmt::Debug;

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;

fn show<T: Debug>(step: &str, result: Result<T, LedgerError>) {
    match result {
        Ok(value) => println!("{step}: {value:?}"),
        Err(error) => println!("{step}: refused ({error:?}: {error})"),
    }
}

fn trial_balance(step: &str, balance: &TrialBalance) {
    println!(
        "{step}: as of {} through entry {} in {} ({} places), {} accounts",
        balance.as_of,
        balance.through_entry_number,
        balance.currency_code,
        balance.minor_units,
        balance.rows.len()
    );
    let units = balance.minor_units;
    for row in &balance.rows {
        println!(
            "  {} {:<20} {:<9} debits {:>10} credits {:>10} balance Dr {:>10} Cr {:>10}",
            row.account.code,
            row.account.name,
            row.account.account_type.as_str(),
            row.debits.shown(units),
            row.credits.shown(units),
            row.debit_balance.shown(units),
            row.credit_balance.shown(units),
        );
    }
    println!(
        "  total Dr {} Cr {}: {}",
        balance.total_debit.shown(units),
        balance.total_credit.shown(units),
        if balance.total_debit == balance.total_credit {
            "equal"
        } else {
            "NOT EQUAL"
        }
    );
}

fn totals(step: &str, totals: &AccountTotals) {
    println!(
        "{step}: {:?} to {:?} through entry {}, {} accounts",
        totals.from.map(|d| d.to_string()),
        totals.to.map(|d| d.to_string()),
        totals.through_entry_number,
        totals.rows.len()
    );
    for row in &totals.rows {
        println!(
            "  {} {:<20} debits {:>10} credits {:>10}",
            row.account.code,
            row.account.name,
            row.debits.shown(totals.minor_units),
            row.credits.shown(totals.minor_units)
        );
    }
}

fn by_owner(step: &str, totals: &OwnerTotals) {
    println!(
        "{step}: through entry {}, {} rows",
        totals.through_entry_number,
        totals.rows.len()
    );
    for row in &totals.rows {
        println!(
            "  {} {:<20} owner {:<28} debits {:>10} credits {:>10}",
            row.account.code,
            row.account.name,
            row.owner_module.as_deref().unwrap_or("(none)"),
            row.debits.shown(totals.minor_units),
            row.credits.shown(totals.minor_units)
        );
    }
}

pub async fn run(db: &Db, services: &Services) -> Outcome<()> {
    let ledger = services.get::<Ledger>(SERVICE)?;
    let manager = sign_in(db, services, 1, 1).await?;
    let viewer = sign_in(db, services, 1, 2).await?;
    let admin = sign_in(db, services, 1, 4).await?;
    let other = sign_in(db, services, 2, 5).await?;
    let contractor = sign_in(db, services, 1, 7).await?;
    let book = ledger.get_book(&manager, 1).await?;
    let money = |text: &str| Money::parse(text, book.minor_units).expect("fixture amount");
    let year_end = date(2026, 12, 31);

    println!("\n== Trial balance (company 1)");
    let first = ledger.trial_balance(&manager, 1, year_end, None).await?;
    trial_balance("manager reads the trial balance", &first);
    let pinned = first.through_entry_number;
    show(
        "viewer (view only) reads company 1's trial balance",
        ledger
            .trial_balance(&viewer, 1, year_end, None)
            .await
            .map(|b| b.rows.len()),
    );
    show(
        "contractor (affiliated with company 2) reads company 1's trial balance",
        ledger
            .trial_balance(&contractor, 1, year_end, None)
            .await
            .map(|b| b.rows.len()),
    );
    show(
        "tenant 2's manager reads company 1's trial balance",
        ledger
            .trial_balance(&other, 1, year_end, None)
            .await
            .map(|b| b.rows.len()),
    );
    show(
        "tenant 2's manager reads tenant 2's company 100 trial balance (no book)",
        ledger
            .trial_balance(&other, 100, year_end, None)
            .await
            .map(|b| b.rows.len()),
    );
    show(
        "manager reads it through entry -1",
        ledger
            .trial_balance(&manager, 1, year_end, Some(-1))
            .await
            .map(|b| b.rows.len()),
    );
    show(
        "manager reads it as of 2025-12-31, before any entry",
        ledger
            .trial_balance(&manager, 1, date(2025, 12, 31), None)
            .await
            .map(|b| (b.through_entry_number, b.rows.len(), b.total_debit)),
    );
    show(
        "manager reads it through entry 0",
        ledger
            .trial_balance(&manager, 1, year_end, Some(0))
            .await
            .map(|b| (b.through_entry_number, b.rows.len())),
    );

    // An entry posted now, dated in the first open period, changes the
    // figures for every date after it; the pin reproduces the earlier ones.
    let year = ledger
        .list_fiscal_years(&manager, 1)
        .await?
        .into_iter()
        .find(|year| year.start_date == date(2026, 1, 1))
        .ok_or("company 1's 2026 fiscal year")?;
    let open = year
        .periods
        .iter()
        .find(|period| period.status == PeriodStatus::Open)
        .ok_or("an open period")?;
    let accounts = ledger.list_accounts(&manager, 1).await?;
    let code = |code: &str| {
        accounts
            .iter()
            .find(|account| account.code == code)
            .map(|account| account.id)
            .ok_or("fixture account")
    };
    let (cash, sales) = (code("1000")?, code("4000")?);
    let backdated = ledger
        .post_entry(
            &manager,
            1,
            &NewEntry {
                entry_date: open.start_date,
                description: "Cash sale recorded after the trial balance".into(),
                reference: None,
                lines: vec![
                    NewLine::debit(cash, money("250.00")),
                    NewLine::credit(sales, money("250.00")),
                ],
            },
            &PostOptions::default(),
        )
        .await?;
    println!(
        "manager posts entry {} dated {}",
        backdated.entry_number, backdated.entry_date
    );
    let now = ledger.trial_balance(&manager, 1, year_end, None).await?;
    let again = ledger
        .trial_balance(&manager, 1, year_end, Some(pinned))
        .await?;
    println!(
        "manager reads it again: through entry {}, total Dr {} (was {})",
        now.through_entry_number, now.total_debit, first.total_debit
    );
    println!(
        "manager reads it through entry {pinned}: the same figures as before: {}",
        again == first
    );

    println!("\n== Account totals (company 1)");
    let june = TotalsQuery {
        from: Some(date(2026, 6, 1)),
        to: Some(date(2026, 6, 30)),
        through_entry_number: Some(pinned),
        exclude_closing: false,
    };
    totals(
        "manager reads June's totals at the pin",
        &ledger.account_totals(&manager, 1, &june).await?,
    );
    let whole = ledger
        .account_totals(
            &manager,
            1,
            &TotalsQuery {
                through_entry_number: Some(pinned),
                ..TotalsQuery::default()
            },
        )
        .await?;
    totals("manager reads every date's totals at the pin", &whole);
    // The same figure from the entries themselves, paged through
    // list_entries at the same pin: PostgreSQL's sum is the lines' sum.
    let mut listed = (Money::ZERO, Money::ZERO);
    let mut after = None;
    loop {
        let page = ledger
            .list_entries(
                &manager,
                1,
                &EntryQuery {
                    account_id: Some(cash),
                    through_entry_number: Some(pinned),
                    after,
                    limit: Some(500),
                    ..EntryQuery::default()
                },
            )
            .await?;
        for line in page.iter().flat_map(|entry| &entry.lines) {
            if line.account_id == cash {
                listed = (listed.0 + line.debit, listed.1 + line.credit);
            }
        }
        match page.last() {
            Some(last) if page.len() == 500 => after = Some((last.entry_date, last.id)),
            _ => break,
        }
    }
    let summed = whole
        .rows
        .iter()
        .find(|row| row.account.id == cash)
        .map(|row| (row.debits, row.credits));
    println!(
        "account 1000's totals equal the sum of its listed lines: {} ({listed:?})",
        summed == Some(listed)
    );
    by_owner(
        "manager reads every date's totals by owner at the pin",
        &ledger
            .account_totals_by_owner(
                &manager,
                1,
                &TotalsQuery {
                    through_entry_number: Some(pinned),
                    ..TotalsQuery::default()
                },
            )
            .await?,
    );
    show(
        "manager reads totals from 2026-07-01 to 2026-06-30",
        ledger
            .account_totals(
                &manager,
                1,
                &TotalsQuery {
                    from: Some(date(2026, 7, 1)),
                    to: Some(date(2026, 6, 30)),
                    ..TotalsQuery::default()
                },
            )
            .await
            .map(|t| t.rows.len()),
    );
    show(
        "manager reads totals by owner through entry -5",
        ledger
            .account_totals_by_owner(
                &manager,
                1,
                &TotalsQuery {
                    through_entry_number: Some(-5),
                    ..TotalsQuery::default()
                },
            )
            .await
            .map(|t| t.rows.len()),
    );
    show(
        "manager reads totals from 2030-01-01 (nothing posted)",
        ledger
            .account_totals(
                &manager,
                1,
                &TotalsQuery {
                    from: Some(date(2030, 1, 1)),
                    ..TotalsQuery::default()
                },
            )
            .await
            .map(|t| (t.through_entry_number, t.rows.len())),
    );
    show(
        "contractor reads company 1's totals by owner",
        ledger
            .account_totals_by_owner(&contractor, 1, &TotalsQuery::default())
            .await
            .map(|t| t.rows.len()),
    );

    println!("\n== Year-end close in the figures (company 2)");
    let closed = ledger
        .list_fiscal_years(&admin, 2)
        .await?
        .into_iter()
        .find(|year| year.status == YearStatus::Closed && year.closing_entry_id.is_some());
    match closed {
        Some(year) => {
            println!(
                "company 2's closed fiscal year {} to {}, closing entry {:?}",
                year.start_date, year.end_date, year.closing_entry_id
            );
            trial_balance(
                "admin reads the trial balance as of its last day",
                &ledger.trial_balance(&admin, 2, year.end_date, None).await?,
            );
            for exclude_closing in [false, true] {
                totals(
                    &format!("admin reads the year's totals, exclude_closing {exclude_closing}"),
                    &ledger
                        .account_totals(
                            &admin,
                            2,
                            &TotalsQuery {
                                from: Some(year.start_date),
                                to: Some(year.end_date),
                                exclude_closing,
                                ..TotalsQuery::default()
                            },
                        )
                        .await?,
                );
            }
        }
        None => println!("company 2 has no closed fiscal year with a closing entry"),
    }

    println!("\n== Evidence (company 1)");
    println!("datasets: {:?}", Ledger::evidence_datasets());
    for dataset in Ledger::evidence_datasets() {
        let mut rows = 0;
        let mut after = None;
        let fields = loop {
            let page = ledger
                .evidence_page(&manager, 1, dataset, None, after)
                .await?;
            rows += page.rows.len();
            match page.next {
                Some(next) => after = Some(next),
                None => break page.fields,
            }
        };
        println!("  {dataset}: {rows} rows of {} fields", fields.len());
    }
    let first_page = ledger
        .evidence_page(&manager, 1, "line", Some(2), None)
        .await?;
    println!(
        "manager reads 2 lines: fields {:?}, next {:?}",
        first_page.fields, first_page.next
    );
    for row in &first_page.rows {
        println!("  {row}");
    }
    let second = ledger
        .evidence_page(&manager, 1, "line", Some(2), first_page.next)
        .await?;
    println!(
        "manager reads the next 2: ids {:?}, next {:?}",
        second
            .rows
            .iter()
            .map(|row| row["id"].clone())
            .collect::<Vec<_>>(),
        second.next
    );
    let entry = ledger
        .evidence_page(&manager, 1, "entry", Some(1), None)
        .await?;
    println!("manager reads 1 entry: {}", entry.rows[0]);
    let fiscal_year = ledger
        .evidence_page(&manager, 1, "fiscal_year", Some(1), None)
        .await?;
    println!("manager reads 1 fiscal year: {}", fiscal_year.rows[0]);
    for (step, dataset, limit, after) in [
        ("an unknown dataset", "entries", None, None),
        ("a limit of 0", "entry", Some(0), None),
        ("a limit of 501", "entry", Some(501), None),
        ("a cursor of -1", "entry", None, Some(-1)),
    ] {
        show(
            &format!("manager reads {step}"),
            ledger
                .evidence_page(&manager, 1, dataset, limit, after)
                .await
                .map(|page| page.rows.len()),
        );
    }
    show(
        "contractor reads company 1's entries",
        ledger
            .evidence_page(&contractor, 1, "entry", None, None)
            .await
            .map(|page| page.rows.len()),
    );
    show(
        "tenant 2's manager reads company 1's entries",
        ledger
            .evidence_page(&other, 1, "entry", None, None)
            .await
            .map(|page| page.rows.len()),
    );
    show(
        "tenant 2's manager reads tenant 2's company 100 entries",
        ledger
            .evidence_page(&other, 100, "entry", None, None)
            .await
            .map(|page| page.rows.len()),
    );
    Ok(())
}
