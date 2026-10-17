//! NT4 running proof: real facade, snapshot, artifact, download and page.
use bilibi_authz::grants::Grants;
use bilibi_database::{pool, Error};
use bilibi_finance_attachments::{Attachments, NewFile};
use bilibi_finance_ledger::{Ledger, Money, NewEntry, NewLine, PostOptions, TotalsQuery};
use bilibi_finance_reports::{Issue, Reports, ReportsError};
use bilibi_finance_walkthrough_fixture::{
    attachments::configure_storage, install, records, sign_in, COMPANY,
};
use bilibi_module_registry::{Contribution, Provenance, Services};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    download::{DownloadRequest, DownloadService},
    page::{LiveInput, LiveRequest, PageRequest, PageService},
};
use jiff::civil::date;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Cursor, Read},
    os::unix::fs::PermissionsExt,
};
bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_attachments_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_receivables_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_payables_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_reconciliation_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_einvoice_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_payroll_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_reports_roles!();
macro_rules! read_role {
    ($module:literal,$cap:expr) => {
        inventory::submit! { Contribution::new(bilibi_authz::VOCABULARY,concat!("walkthrough.fixture.auditor-pack.",$module),Provenance { module:$module,source:file!(),line:line!() },&bilibi_authz::VocabularyContribution {
            domains:&[],verbs:&[],capabilities:&[],system_principals:&[],roles:&[bilibi_authz::RoleDefinition {code:"manager",name:"manager",description:"Walkthrough auditor",grant_all:false,capabilities:&[$cap]}],
        }) }
    };
}
read_role!("finance/sst", bilibi_finance_sst::VIEW);
read_role!("finance/fixed_assets", bilibi_finance_fixed_assets::VIEW);
type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
async fn setting(
    db: &bilibi_database::Db,
    services: &Services,
    admin: &Scope,
    key: &str,
    value: Value,
) -> Outcome<()> {
    let mut client = db.get().await?;
    let boundary = services.get::<bilibi_database::MutationBoundary>("base/database.mutations")?;
    let mut tx = boundary
        .begin(
            &mut client,
            admin.mutation_context("auditor-pack.fixture.configure"),
        )
        .await?;
    services
        .get::<Settings>("base/settings.store")?
        .put(&mut tx, admin, Target::Global, key, value)
        .await?;
    tx.commit().await?;
    Ok(())
}
fn entry(debit: i64, credit: i64, amount: Money) -> NewEntry {
    NewEntry {
        entry_date: date(2026, 3, 15),
        description: " \t=auditor,\"quoted\"\nsecond line".into(),
        reference: None,
        lines: vec![
            NewLine::debit(debit, amount),
            NewLine::credit(credit, amount),
        ],
    }
}
fn inspect(bytes: &[u8]) -> Outcome<(Value, HashMap<String, Vec<u8>>)> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut files = HashMap::new();
    for i in 0..zip.len() {
        let mut file = zip.by_index(i)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        files.insert(file.name()?.into_owned(), bytes);
    }
    let manifest: Value = serde_json::from_slice(&files["manifest.json"])?;
    assert_eq!(
        files.len(),
        manifest["files"].as_object().unwrap().len() + 1
    );
    for (name, meta) in manifest["files"].as_object().unwrap() {
        let sha: String = Sha256::digest(&files[name])
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(meta["sha256"], sha, "bad hash {name}");
    }
    assert_eq!(
        manifest["control_totals"]["debits"],
        manifest["control_totals"]["credits"]
    );
    Ok((manifest, files))
}
#[tokio::main]
async fn main() -> Outcome<()> {
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    bilibi_database::migrate(&db, &bilibi_module_registry::discover()?).await?;
    let app = bilibi_web::App::new(db.clone(), "http://127.0.0.1:4324")?;
    let services = &app.services;
    let installed = install(&db, services).await?;
    let manager = sign_in(&db, services, 1, 1).await?;
    let admin = sign_in(&db, services, 1, 4).await?;
    let viewer = sign_in(&db, services, 1, 2).await?;
    let other = sign_in(&db, services, 2, 5).await?;
    let ledger = services.get::<Ledger>(bilibi_finance_ledger::SERVICE)?;
    let reports = services.get::<Reports>(bilibi_finance_reports::SERVICE)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    let root = std::env::var("AUDITOR_PACK_PROOF_DIR")?;
    std::fs::create_dir_all(&root)?;
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
    let storage = format!("{root}/artifacts");
    std::fs::create_dir_all(&storage)?;
    std::fs::set_permissions(&storage, std::fs::Permissions::from_mode(0o700))?;
    assert!(matches!(
        reports
            .export_auditor_pack(&manager, COMPANY, date(2026, 3, 31))
            .await,
        Err(ReportsError::Artifact("policy"))
    ));
    configure_storage(&db, services, &admin, &storage).await?;
    let debit = installed
        .accounts
        .iter()
        .find(|a| a.code == "1000")
        .unwrap()
        .id;
    let credit = installed
        .accounts
        .iter()
        .find(|a| a.code == "4000")
        .unwrap()
        .id;
    let amount = Money::parse("1.23", installed.book.minor_units)?;
    let mut client = db.get().await?;
    let mut tx = ledger
        .write(
            &mut client,
            &manager,
            COMPANY,
            &[bilibi_finance_ledger::POST_ENTRIES],
            "auditor-pack.fixture.entries",
        )
        .await?;
    for _ in 0..502 {
        ledger
            .post_entry_in(
                &mut tx,
                &manager,
                COMPANY,
                &entry(debit, credit, amount),
                &PostOptions::default(),
            )
            .await?;
    }
    tx.commit().await?;
    drop(client);
    let record = records::add(
        &db,
        1,
        COMPANY,
        "open",
        "Auditor source",
        date(2026, 3, 15),
        false,
    )
    .await?;
    let attachment = services
        .get::<Attachments>(bilibi_finance_attachments::SERVICE)?
        .attach(
            &manager,
            COMPANY,
            records::OPEN_SOURCE,
            record,
            &NewFile {
                file_name: "=supporting.csv".into(),
                content: b"a,b\r\n1,2\r\n".to_vec(),
                content_type: Some("text/csv".into()),
            },
        )
        .await?;
    reports
        .issue_report(
            &manager,
            COMPANY,
            &Issue::TrialBalance(date(2026, 3, 31)),
            None,
        )
        .await?;
    let mut client = db.get().await?;
    let snapshot = (**client)
        .build_transaction()
        .isolation_level(tokio_postgres::IsolationLevel::RepeatableRead)
        .read_only(true)
        .start()
        .await?;
    let first = ledger
        .evidence_page_on(&snapshot, &manager, COMPANY, "entry", Some(500), None)
        .await?;
    let pin = ledger
        .account_totals_on(&snapshot, &manager, COMPANY, &TotalsQuery::default())
        .await?
        .through_entry_number;
    ledger
        .post_entry(
            &manager,
            COMPANY,
            &entry(debit, credit, amount),
            &PostOptions::default(),
        )
        .await?;
    ledger
        .update_account(
            &manager,
            COMPANY,
            debit,
            &bilibi_finance_ledger::AccountChange {
                name: Some("Changed after snapshot".into()),
                ..Default::default()
            },
        )
        .await?;
    let old_chart = ledger
        .list_accounts_on(&snapshot, &manager, COMPANY)
        .await?;
    assert_ne!(
        old_chart.iter().find(|a| a.id == debit).unwrap().name,
        "Changed after snapshot"
    );
    let rest = ledger
        .evidence_page_on(&snapshot, &manager, COMPANY, "entry", Some(500), first.next)
        .await?;
    assert_eq!(first.rows.len() + rest.rows.len(), 502);
    assert_eq!(
        ledger
            .account_totals_on(&snapshot, &manager, COMPANY, &TotalsQuery::default())
            .await?
            .through_entry_number,
        pin
    );
    snapshot.commit().await?;
    drop(client);
    println!("Snapshot pages exclude a concurrent committed backdated entry");
    let artifact = reports
        .export_auditor_pack(&manager, COMPANY, date(2026, 3, 31))
        .await?;
    let pack = reports
        .read_auditor_pack(&manager, COMPANY, &artifact.id)
        .await?;
    let (manifest, files) = inspect(&pack.bytes)?;
    assert_eq!(manifest["through_entry_number"], pin + 1);
    for (name, count) in [
        ("ledger-entry.csv", 503),
        ("ledger-line.csv", 1006),
        ("issued-reports.csv", 1),
        ("attachments-attachment.csv", 1),
    ] {
        assert_eq!(manifest["files"][name]["row_count"], count);
    }
    let index = String::from_utf8(files["attachments-attachment.csv"].clone())?;
    assert!(index.contains(&attachment.sha256));
    assert!(index.contains("'=supporting.csv"));
    assert!(String::from_utf8(files["general-ledger.csv"].clone())?.contains("' \t=auditor"));
    std::fs::write(format!("{root}/auditor-pack.zip"), &pack.bytes)?;
    std::fs::write(
        format!("{root}/manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    std::fs::write(format!("{root}/artifact-id"), &artifact.id)?;
    println!(
        "{} archive members; hashes, GL totals, multi-page evidence and formula text checked",
        files.len()
    );
    assert!(reports
        .read_auditor_pack(&other, COMPANY, &artifact.id)
        .await
        .is_err());
    assert!(reports
        .read_auditor_pack(&manager, 2, &artifact.id)
        .await
        .is_err());
    assert!(reports
        .export_auditor_pack(&viewer, COMPANY, date(2026, 3, 31))
        .await
        .is_err());
    assert!(reports
        .read_auditor_pack(&manager, COMPANY, "not-an-artifact")
        .await
        .is_err());
    let downloads = services.get::<DownloadService<Scope, Error>>("finance/reports.downloads")?;
    let paths = HashMap::from([("artifact_id".into(), artifact.id.clone())]);
    let empty = HashMap::new();
    let file = downloads
        .0
        .serve(DownloadRequest {
            operation: "auditor-pack",
            scope: &manager,
            path: &paths,
            query: &empty,
        })
        .await?;
    assert_eq!(file.bytes, pack.bytes);
    assert!(file
        .headers()
        .iter()
        .any(|(k, v)| *k == "cache-control" && v == "private, no-store"));
    let pages = services.get::<PageService<Scope, Error>>("finance/reports.pages")?;
    let query = HashMap::from([
        ("period".into(), "custom".into()),
        ("from".into(), "2026-03-01".into()),
        ("to".into(), "2026-03-31".into()),
        ("through".into(), "1".into()),
    ]);
    let mut state = pages
        .0
        .mount(PageRequest {
            operation: "reports",
            scope: Some(&manager),
            path: &empty,
            query: &query,
            body: &[],
            framed: false,
        })
        .await?
        .unwrap();
    state
        .update(LiveRequest {
            scope: &manager,
            credential: "",
            input: LiveInput::Refresh,
        })
        .await?;
    let event = json!({"type":"export_auditor_pack"});
    let body = state
        .update(LiveRequest {
            scope: &manager,
            credential: "",
            input: LiveInput::Event(&event),
        })
        .await?;
    assert!(body.html.contains("Auditor pack generated."));
    assert!(body.html.contains("download-auditor-pack"));
    for cap in [
        bilibi_finance_reports::EXPORT,
        bilibi_finance_ledger::VIEW,
        bilibi_finance_attachments::VIEW,
        bilibi_finance_receivables::VIEW,
        bilibi_finance_payables::VIEW,
        bilibi_finance_reconciliation::VIEW,
        bilibi_finance_einvoice::VIEW,
        bilibi_finance_payroll::VIEW,
        bilibi_finance_sst::VIEW,
        bilibi_finance_fixed_assets::VIEW,
    ] {
        grants.set_direct(&admin, 1, cap, Some(false)).await?;
        assert!(
            matches!(
                reports
                    .read_auditor_pack(&manager, COMPANY, &artifact.id)
                    .await,
                Err(ReportsError::Unauthorized)
            ),
            "read did not refuse {cap}"
        );
        assert!(matches!(
            downloads
                .0
                .serve(DownloadRequest {
                    operation: "auditor-pack",
                    scope: &manager,
                    path: &paths,
                    query: &empty
                })
                .await,
            Err(Error::Forbidden)
        ));
        assert!(reports
            .export_auditor_pack(&manager, COMPANY, date(2026, 3, 31))
            .await
            .is_err());
        let body = state
            .update(LiveRequest {
                scope: &manager,
                credential: "",
                input: LiveInput::Event(&event),
            })
            .await?;
        assert!(body
            .html
            .contains("Auditor packs require read access to every Finance module."));
        grants.set_direct(&admin, 1, cap, None).await?;
    }
    println!("Every revoked source grant/export blocks facade, download and already-mounted page");
    setting(&db, services, &admin, "artifacts.max_bytes", json!(1)).await?;
    assert!(matches!(
        reports
            .export_auditor_pack(&manager, COMPANY, date(2026, 3, 31))
            .await,
        Err(ReportsError::InvalidSize)
    ));
    setting(
        &db,
        services,
        &admin,
        "artifacts.max_bytes",
        json!(10_485_760),
    )
    .await?;
    if std::env::args().any(|v| v == "serve") {
        println!(
            "Serving http://127.0.0.1:4324; manager@example.test with the walkthrough password"
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:4324").await?;
        axum::serve(listener, bilibi_web::router(app)).await?;
    }
    Ok(())
}
