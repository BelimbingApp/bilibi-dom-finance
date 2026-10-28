//! Disposable conversion proof through the real facade, never product data.
//! Run on a fresh database: cargo run -p bilibi-finance-walkthrough-fixture
//! --bin opening-balances. UI proof: ledger-ui opening. MCP proof: the
//! agent-operations credential fixture and opening-balances-operations.py.
use bilibi_database::pool;
use bilibi_finance_ledger::{Ledger, LedgerError, OpeningBalances, Reversal};
use bilibi_finance_walkthrough_fixture::{install, services, sign_in, COMPANY};
use jiff::civil::date;

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_database::migrate(&db, &modules).await?;
    let services = services(&db, &modules)?;
    install(&db, &services).await?;
    let manager = sign_in(&db, &services, 1, 1).await?;
    let reviewer = sign_in(&db, &services, 1, 4).await?;
    let viewer = sign_in(&db, &services, 1, 2).await?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let mut source = OpeningBalances {
        conversion_date: date(2026, 7, 1), currency_code: "MYR".into(),
        source_reference: "SOURCE-CLOSE-2026-06-30".into(),
        csv: "source_id,account_code,debit,credit\nbank,1000,10000,\nreceivables,1100,3000,\ncost,5000,2000,\npayables,2000,,1500\ncapital,3000,,8000\nsales,4000,,5500\n".into(),
    };
    let original = source.csv.clone();
    let mut invalids = 0;
    for csv in [
        original.replace("5500", "5501"),
        original.replace("1000,", "unknown,"),
        original.replace("receivables,", "bank,"),
        original.replace("10000,", "10000.001,"),
        original.replace("10000,", "-10000,"),
        original.replace("10000,", "10000,2"),
        "source_id,account_code,debit,credit\n\"open".into(),
    ] {
        source.csv = csv;
        assert!(matches!(
            ledger
                .submit_opening_balances(&manager, COMPANY, &source)
                .await,
            Err(LedgerError::Invalid(_))
        ));
        invalids += 1;
    }
    source.csv = original;
    source.currency_code = "USD".into();
    assert!(matches!(
        ledger
            .submit_opening_balances(&manager, COMPANY, &source)
            .await,
        Err(LedgerError::Invalid(_))
    ));
    source.currency_code = "MYR".into();
    assert!(matches!(
        ledger
            .submit_opening_balances(&viewer, COMPANY, &source)
            .await,
        Err(LedgerError::Unauthorized)
    ));
    assert_eq!(
        ledger
            .trial_balance(&manager, COMPANY, source.conversion_date, None)
            .await?
            .through_entry_number,
        0
    );
    println!(
        "opening: {invalids} malformed, unmapped, precision or unbalanced files, wrong currency and viewer refused; no posting"
    );

    // The company lock makes two independent retries one source submission.
    let (a, b) = tokio::join!(
        ledger.submit_opening_balances(&manager, COMPANY, &source),
        ledger.submit_opening_balances(&manager, COMPANY, &source)
    );
    let journal = a?;
    assert_eq!(journal.id, b?.id);
    assert_eq!(
        ledger
            .trial_balance(&manager, COMPANY, source.conversion_date, None)
            .await?
            .through_entry_number,
        0
    );
    assert!(matches!(
        ledger.approve_journal(&manager, COMPANY, journal.id).await,
        Err(LedgerError::SamePerson)
    ));
    let rejected = ledger
        .reject_journal(
            &reviewer,
            COMPANY,
            journal.id,
            "Source closing report superseded before approval",
        )
        .await?;
    assert_eq!(
        rejected.id,
        ledger
            .submit_opening_balances(&manager, COMPANY, &source)
            .await?
            .id
    );
    source.source_reference.push_str("-REVISED");
    let journal = ledger
        .submit_opening_balances(&manager, COMPANY, &source)
        .await?;
    let posted = ledger
        .approve_journal(&reviewer, COMPANY, journal.id)
        .await?;
    let entry_id = posted
        .decision
        .as_ref()
        .and_then(|d| d.entry_id)
        .ok_or("approved entry")?;
    let entry = ledger.get_entry(&manager, COMPANY, entry_id).await?;
    assert_eq!(entry.lines.len(), 6);
    let trial = ledger
        .trial_balance(&manager, COMPANY, source.conversion_date, None)
        .await?;
    assert_eq!(trial.total_debit.shown(trial.minor_units), "15000.00");
    assert_eq!(trial.total_credit, trial.total_debit);
    for (code, debit, credit) in [
        ("1000", "10000.00", "0.00"),
        ("1100", "3000.00", "0.00"),
        ("5000", "2000.00", "0.00"),
        ("2000", "0.00", "1500.00"),
        ("3000", "0.00", "8000.00"),
        ("4000", "0.00", "5500.00"),
    ] {
        let row = trial
            .rows
            .iter()
            .find(|r| r.account.code == code)
            .ok_or("account balance")?;
        assert_eq!(row.debit_balance.shown(trial.minor_units), debit);
        assert_eq!(row.credit_balance.shown(trial.minor_units), credit);
    }
    assert_eq!(
        ledger
            .submit_opening_balances(&manager, COMPANY, &source)
            .await?
            .id,
        journal.id
    );
    source.csv = source.csv.replace("10000", "10001");
    assert!(matches!(
        ledger
            .submit_opening_balances(&manager, COMPANY, &source)
            .await,
        Err(LedgerError::Invalid(_))
    ));
    source.csv = source.csv.replace("10001", "10000");
    source.source_reference.push_str("-DUPLICATE");
    assert!(matches!(
        ledger
            .submit_opening_balances(&manager, COMPANY, &source)
            .await,
        Err(LedgerError::Invalid(_))
    ));
    println!(
        "opening: racing retries share a journal; same-person approval refused; rejected source replaced with a new reference; six account balances reconcile to 15000.00; posted retry creates no entry"
    );

    ledger
        .reverse_entry(
            &manager,
            COMPANY,
            entry_id,
            &Reversal {
                entry_date: source.conversion_date,
                description: "Correct conversion figures against the revised source closing report"
                    .into(),
                reference: Some("CORRECTION-1".into()),
            },
        )
        .await?;
    assert_eq!(
        ledger
            .trial_balance(&manager, COMPANY, source.conversion_date, None)
            .await?
            .total_debit
            .shown(trial.minor_units),
        "0.00"
    );
    let corrected = ledger
        .submit_journal(
            &manager,
            COMPANY,
            &bilibi_finance_ledger::NewEntry {
                entry_date: source.conversion_date,
                description: "Corrected opening balances".into(),
                reference: Some("CORRECTION-2".into()),
                lines: entry
                    .lines
                    .iter()
                    .map(|l| bilibi_finance_ledger::NewLine {
                        account_id: l.account_id,
                        debit: l.debit,
                        credit: l.credit,
                        description: l.description.clone(),
                    })
                    .collect(),
            },
        )
        .await?;
    ledger
        .approve_journal(&reviewer, COMPANY, corrected.id)
        .await?;
    assert_eq!(
        ledger
            .trial_balance(&manager, COMPANY, source.conversion_date, None)
            .await?
            .total_debit,
        trial.total_debit
    );
    let evidence = ledger
        .evidence_page(&manager, COMPANY, "opening_balances", Some(500), None)
        .await?;
    assert_eq!(evidence.rows.len(), 2);
    assert_eq!(
        evidence.rows[1]["source_csv"].as_str(),
        Some(source.csv.as_str())
    );
    let client = db.get().await?;
    let audited: i64 = client.query_one("SELECT count(*) FROM base_audit_mutations WHERE auditable_type='finance.ledger.opening_balances'", &[]).await?.get(0);
    assert_eq!(audited, 2);
    assert!(client.execute("UPDATE finance_ledger_opening_balances SET source_csv='overwritten' WHERE tenant_id=1 AND company_id=1", &[]).await.is_err());
    println!("opening: reversal and independently approved correction restore source balances; two source evidence records and their audit changes retained; database refuses evidence overwrite");
    println!(
        "EXCLUDED: settleable open invoices/bills, aging and control tie-outs, bank reconciliation items, asset registers, tax return detail, historical/comparative migration; no complete conversion claim"
    );
    Ok(())
}
