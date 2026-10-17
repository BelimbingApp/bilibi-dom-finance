//! The e-invoice section of the walkthrough: the module's facade answering
//! what Bilimbi's does (code lists, overrides, setup), the company settings
//! with their validators and encryption, the signing certificate and key
//! validators, and the database's own refusals. Every line is the actual
//! answer; nothing here is a test double. A throwaway certificate and key are
//! made with `openssl` for the run and removed after it.
use bilibi_database::{Db, MutationBoundary};
use bilibi_finance_einvoice::{
    Authority, CodeOverrideChange, Einvoice, EinvoiceError, FormatKind, SERVICE,
};
use bilibi_module_registry::Services;
use bilibi_party_ids::OrganisationId;
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use serde_json::{json, Value};
use std::{fmt::Debug, process::Command};

fn show<T: Debug>(step: &str, result: Result<T, EinvoiceError>) {
    match result {
        Ok(value) => println!("{step}: {value:?}"),
        Err(EinvoiceError::Invalid(errors)) => {
            let errors: Vec<String> = errors
                .iter()
                .map(|error| format!("{} {}", error.field, error.message))
                .collect();
            println!("{step}: refused ({})", errors.join("; "));
        }
        Err(error) => println!("{step}: refused ({error:?})"),
    }
}

/// The people the section acts as.
pub struct Actors<'a> {
    pub manager: &'a Scope,
    pub viewer: &'a Scope,
    pub contractor: &'a Scope,
    pub other_tenant: &'a Scope,
}

/// Saves one company setting as the Settings screen would, for the manager.
async fn save(
    db: &Db,
    services: &Services,
    scope: &Scope,
    company: i64,
    key: &str,
    value: Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let mutations = services.get::<MutationBoundary>("base/database.mutations")?;
    let settings = services.get::<Settings>("base/settings.store")?;
    let mut client = db.get().await?;
    let mut tx = mutations
        .begin(
            &mut client,
            scope.mutation_context("finance.einvoice.walkthrough.settings.save"),
        )
        .await?;
    let target = Target::Company(OrganisationId::new(company)?);
    let outcome = settings.put(&mut tx, scope, target, key, value).await;
    match outcome {
        Ok(()) => {
            tx.commit().await?;
            Ok("stored".into())
        }
        Err(error) => {
            tx.rollback().await?;
            Ok(format!("refused ({error:?})"))
        }
    }
}

/// A throwaway RSA certificate and key, as PEM files `openssl` writes into a
/// temporary directory that is removed on return.
struct Pems {
    certificate: String,
    key_pkcs8: String,
    key_pkcs1: String,
    key_encrypted: String,
    key_ec: String,
}
fn make_pems() -> Result<Pems, Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("einvoice-walkthrough-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
    let run = |args: &[&str]| -> Result<(), Box<dyn std::error::Error>> {
        let status = Command::new("openssl").args(args).output()?.status;
        status
            .success()
            .then_some(())
            .ok_or_else(|| "openssl failed".into())
    };
    let (cert, key, trad, enc, ec) = (
        path("cert.pem"),
        path("key.pem"),
        path("trad.pem"),
        path("enc.pem"),
        path("ec.pem"),
    );
    run(&[
        "req",
        "-x509",
        "-newkey",
        "rsa:2048",
        "-nodes",
        "-days",
        "30",
        "-subj",
        "/C=MY/O=Walkthrough Sdn Bhd/CN=Walkthrough",
        "-keyout",
        &key,
        "-out",
        &cert,
    ])?;
    run(&["rsa", "-in", &key, "-traditional", "-out", &trad])?;
    run(&[
        "pkcs8",
        "-topk8",
        "-in",
        &key,
        "-passout",
        "pass:walkthrough",
        "-out",
        &enc,
    ])?;
    run(&[
        "ecparam",
        "-genkey",
        "-name",
        "prime256v1",
        "-noout",
        "-out",
        &ec,
    ])?;
    let read = |file: &str| std::fs::read_to_string(file);
    let pems = Pems {
        certificate: read(&cert)?,
        key_pkcs8: read(&key)?,
        key_pkcs1: read(&trad)?,
        key_encrypted: read(&enc)?,
        key_ec: read(&ec)?,
    };
    std::fs::remove_dir_all(&dir)?;
    Ok(pems)
}

pub async fn prove(
    db: &Db,
    services: &Services,
    actors: &Actors<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let einvoice = services.get::<Einvoice>(SERVICE)?;
    let (manager, viewer, contractor, other) = (
        actors.manager,
        actors.viewer,
        actors.contractor,
        actors.other_tenant,
    );
    let company = 1;

    println!("\n== E-invoice code lists");
    println!("the authority publishes: {:?}", Einvoice::code_lists());
    for list in Einvoice::code_lists() {
        let codes = einvoice.list_codes(manager, company, list).await?;
        println!("  {list}: {} codes", codes.len());
    }
    show(
        "manager reads the document types",
        einvoice.list_codes(manager, company, "document_type").await,
    );
    show(
        "manager reads an unknown list",
        einvoice.list_codes(manager, company, "colours").await,
    );
    show(
        "viewer (no affiliation, no tenant-wide reach) reads a list",
        einvoice.list_codes(viewer, company, "state").await,
    );
    show(
        "contractor (affiliated with company 2) reads company 1's list",
        einvoice.list_codes(contractor, company, "state").await,
    );
    show(
        "tenant 2's manager reads company 1's list",
        einvoice.list_codes(other, company, "state").await,
    );
    show(
        "manager (tenant-wide reach) reads company 2's states",
        einvoice
            .list_codes(manager, 2, "state")
            .await
            .map(|c| c.len()),
    );

    println!("\n== Code overrides");
    let rename = CodeOverrideChange {
        description: Some(Some("  Wilayah Persekutuan KL  ".into())),
        active: None,
    };
    show(
        "manager renames state 14",
        einvoice
            .override_code(manager, company, "state", "14", &rename)
            .await,
    );
    show(
        "company 1 now reads state 14",
        einvoice
            .list_codes(manager, company, "state")
            .await
            .map(|codes| codes.get("14").cloned()),
    );
    show(
        "company 2 still reads state 14",
        einvoice
            .list_codes(manager, 2, "state")
            .await
            .map(|codes| codes.get("14").cloned()),
    );
    show(
        "manager renames state 14 to the same name again (nothing changes)",
        einvoice
            .override_code(manager, company, "state", "14", &rename)
            .await,
    );
    show(
        "manager retires state 17",
        einvoice
            .override_code(
                manager,
                company,
                "state",
                "17",
                &CodeOverrideChange {
                    description: None,
                    active: Some(false),
                },
            )
            .await,
    );
    show(
        "company 1's states after retiring 17",
        einvoice
            .list_codes(manager, company, "state")
            .await
            .map(|codes| (codes.len(), codes.contains_key("17"))),
    );
    show(
        "manager adds state 99 the shipped list lacks",
        einvoice
            .override_code(
                manager,
                company,
                "state",
                "99",
                &CodeOverrideChange {
                    description: Some(Some("New territory".into())),
                    active: None,
                },
            )
            .await,
    );
    show(
        "manager adds state 98 with no description (it is named by its code)",
        einvoice
            .override_code(
                manager,
                company,
                "state",
                "98",
                &CodeOverrideChange {
                    description: None,
                    active: Some(true),
                },
            )
            .await,
    );
    show(
        "company 1's state 98 reads as its code",
        einvoice
            .list_codes(manager, company, "state")
            .await
            .map(|codes| codes.get("98").cloned()),
    );
    show(
        "manager un-renames state 14 (clears the description)",
        einvoice
            .override_code(
                manager,
                company,
                "state",
                "14",
                &CodeOverrideChange {
                    description: Some(None),
                    active: None,
                },
            )
            .await,
    );
    show(
        "manager overrides a code with a bad shape",
        einvoice
            .override_code(manager, company, "state", "not a code!", &rename)
            .await,
    );
    show(
        "manager overrides a code with a 501-character description",
        einvoice
            .override_code(
                manager,
                company,
                "state",
                "20",
                &CodeOverrideChange {
                    description: Some(Some("x".repeat(501))),
                    active: None,
                },
            )
            .await,
    );
    show(
        "manager overrides a code in an unknown list",
        einvoice
            .override_code(manager, company, "colours", "1", &rename)
            .await,
    );
    show(
        "staff (contractor, company 2) overrides a company 1 code",
        einvoice
            .override_code(contractor, company, "state", "14", &rename)
            .await,
    );
    show(
        "viewer overrides a code (view only)",
        einvoice
            .override_code(viewer, company, "state", "14", &rename)
            .await,
    );

    println!("\n== Setup before anything is configured");
    show(
        "manager reads company 1's setup",
        einvoice.setup(manager, company).await,
    );
    show(
        "viewer reads company 1's setup (view is not settings.manage)",
        einvoice.setup(viewer, company).await,
    );

    println!("\n== Company settings: validators");
    let settings = [
        (
            "environment: 'staging'",
            "finance.einvoice.environment",
            json!("staging"),
        ),
        (
            "environment: 'production'",
            "finance.einvoice.environment",
            json!("production"),
        ),
        (
            "expiry_alert_days: 0",
            "finance.einvoice.expiry_alert_days",
            json!(0),
        ),
        (
            "expiry_alert_days: 400",
            "finance.einvoice.expiry_alert_days",
            json!(400),
        ),
        (
            "expiry_alert_days: 45",
            "finance.einvoice.expiry_alert_days",
            json!(45),
        ),
        (
            "document_version: '2.0'",
            "finance.einvoice.document_version",
            json!("2.0"),
        ),
        (
            "document_version: '1.0'",
            "finance.einvoice.document_version",
            json!("1.0"),
        ),
        (
            "secret_expires_on: 'soon'",
            "finance.einvoice.production.secret_expires_on",
            json!("soon"),
        ),
        (
            "posting.receivable_account: 51 characters",
            "finance.einvoice.posting.receivable_account",
            json!("1".repeat(51)),
        ),
        (
            "posting.receivable_account: '1100'",
            "finance.einvoice.posting.receivable_account",
            json!("1100"),
        ),
        (
            "posting.revenue_account: '4000'",
            "finance.einvoice.posting.revenue_account",
            json!("4000"),
        ),
        (
            "posting.tax_account: '2100'",
            "finance.einvoice.posting.tax_account",
            json!("2100"),
        ),
        (
            "supplier.tax_id: 'C1234'",
            "finance.einvoice.supplier.tax_id",
            json!("C1234"),
        ),
        (
            "supplier.tax_id: 'C2584563222'",
            "finance.einvoice.supplier.tax_id",
            json!("C2584563222"),
        ),
        (
            "supplier.registration_type: 'PASS'",
            "finance.einvoice.supplier.registration_type",
            json!("PASS"),
        ),
        (
            "supplier.industry_code: '4711'",
            "finance.einvoice.supplier.industry_code",
            json!("4711"),
        ),
        (
            "supplier.industry_code: '47111'",
            "finance.einvoice.supplier.industry_code",
            json!("47111"),
        ),
        (
            "supplier.phone: '0123456789'",
            "finance.einvoice.supplier.phone",
            json!("0123456789"),
        ),
        (
            "supplier.phone: '+60123456789'",
            "finance.einvoice.supplier.phone",
            json!("+60123456789"),
        ),
        (
            "supplier.state_code: '1'",
            "finance.einvoice.supplier.state_code",
            json!("1"),
        ),
        (
            "supplier.state_code: '14'",
            "finance.einvoice.supplier.state_code",
            json!("14"),
        ),
        (
            "supplier.country_code: 'My'",
            "finance.einvoice.supplier.country_code",
            json!("My"),
        ),
        (
            "supplier.country_code: 'MYS'",
            "finance.einvoice.supplier.country_code",
            json!("MYS"),
        ),
        (
            "supplier.registration_number: 21 characters",
            "finance.einvoice.supplier.registration_number",
            json!("9".repeat(21)),
        ),
        (
            "supplier.registration_number: '201901234567'",
            "finance.einvoice.supplier.registration_number",
            json!("201901234567"),
        ),
        (
            "supplier.business_activity: blank",
            "finance.einvoice.supplier.business_activity",
            json!("   "),
        ),
        (
            "supplier.business_activity: 'Retail'",
            "finance.einvoice.supplier.business_activity",
            json!("Retail"),
        ),
        (
            "supplier.address_line1: '1 Jalan Contoh'",
            "finance.einvoice.supplier.address_line1",
            json!("1 Jalan Contoh"),
        ),
        (
            "supplier.city: 'Kuala Lumpur'",
            "finance.einvoice.supplier.city",
            json!("Kuala Lumpur"),
        ),
        (
            "an undeclared key",
            "finance.einvoice.supplier.nickname",
            json!("x"),
        ),
    ];
    for (step, key, value) in settings {
        println!(
            "{step}: {}",
            save(db, services, manager, company, key, value).await?
        );
    }

    println!("\n== Signing certificate and private key: PEM validators");
    let pems = make_pems()?;
    let certificate_key = "finance.einvoice.certificate_pem";
    let private_key = "finance.einvoice.private_key_pem";
    let cases = [
        ("certificate: not PEM", certificate_key, "hello".to_owned()),
        (
            "certificate: a private key",
            certificate_key,
            pems.key_pkcs8.clone(),
        ),
        (
            "certificate: truncated",
            certificate_key,
            pems.certificate[..120].to_owned(),
        ),
        ("private key: not PEM", private_key, "hello".to_owned()),
        (
            "private key: a certificate",
            private_key,
            pems.certificate.clone(),
        ),
        (
            "private key: encrypted PKCS#8",
            private_key,
            pems.key_encrypted.clone(),
        ),
        ("private key: an EC key", private_key, pems.key_ec.clone()),
        (
            "private key: two keys",
            private_key,
            format!("{}{}", pems.key_pkcs8, pems.key_pkcs8),
        ),
        (
            "certificate: a valid X.509 certificate",
            certificate_key,
            pems.certificate.clone(),
        ),
        (
            "private key: a valid PKCS#8 RSA key",
            private_key,
            pems.key_pkcs8.clone(),
        ),
        (
            "private key: a valid PKCS#1 RSA key",
            private_key,
            pems.key_pkcs1.clone(),
        ),
    ];
    for (step, key, pem) in cases {
        println!(
            "{step}: {}",
            save(db, services, manager, company, key, json!(pem)).await?
        );
    }

    println!("\n== Encrypted settings");
    println!(
        "production.client_secret: {}",
        save(
            db,
            services,
            manager,
            company,
            "finance.einvoice.production.client_secret",
            json!("walkthrough-client-secret"),
        )
        .await?
    );
    let client = db.get().await?;
    for key in [
        "finance.einvoice.production.client_secret",
        certificate_key,
        private_key,
        "finance.einvoice.document_version",
    ] {
        let row = client
            .query_one(
                "SELECT is_encrypted, value::text LIKE '%walkthrough-client-secret%' OR value::text LIKE '%BEGIN%', length(value::text) FROM base_settings WHERE key=$1 AND scope_type='company' AND scope_id=1",
                &[&key],
            )
            .await?;
        println!(
            "stored {key}: encrypted={} plaintext visible in the row={} ({} bytes)",
            row.get::<_, bool>(0),
            row.get::<_, bool>(1),
            row.get::<_, i32>(2)
        );
    }
    let audited: i64 = client
        .query_one(
            "SELECT count(*) FROM base_audit_mutations WHERE auditable_type='base/settings' AND (new_values::text LIKE '%walkthrough-client-secret%' OR new_values::text LIKE '%BEGIN%')",
            &[],
        )
        .await?
        .get(0);
    println!("audit rows that carry a secret in the clear: {audited}");

    println!("\n== Setup after configuring");
    show(
        "manager reads company 1's setup",
        einvoice.setup(manager, company).await,
    );
    show(
        "manager reads company 2's setup (nothing configured there)",
        einvoice.setup(manager, 2).await,
    );

    println!("\n== The authority's shapes and rules");
    let myinvois = bilibi_finance_einvoice_authority();
    for (kind, value) in [
        (FormatKind::TaxId, "C2584563222"),
        (FormatKind::TaxId, "EI00000000010"),
        (FormatKind::TaxId, "c2584563222"),
        (FormatKind::TaxId, "ABC2584563222"),
        (FormatKind::Phone, "+60123456789"),
        (FormatKind::Phone, "60123456789"),
        (FormatKind::RegistrationType, "NRIC"),
        (FormatKind::RegistrationType, "nric"),
    ] {
        println!("{kind:?} {value:?}: {}", myinvois.valid_format(kind, value));
    }
    for document_type in ["01", "02", "03", "04", "11", "12", "13", "14", "99"] {
        println!(
            "{document_type} {}: {:?}",
            myinvois.document_type_name(document_type),
            myinvois.posting_rule(document_type)
        );
    }
    println!(
        "issuable {:?}; notes {:?}; self-billed {:?}",
        myinvois.issuable_document_types(),
        myinvois.note_document_types(),
        myinvois.self_billed_document_types()
    );
    println!(
        "versions {:?}; cancellation window {}h; outage notice {}h",
        myinvois.document_versions(),
        myinvois.cancellation_window_hours(),
        myinvois.outage_notice_hours()
    );
    println!(
        "consolidation window for October 2026: {:?}",
        myinvois.consolidation_window(jiff::civil::date(2026, 10, 17))
    );
    println!(
        "validation url: {}",
        myinvois.validation_url("https://preprod.myinvois.hasil.gov.my/", "UUID", "LONGID")
    );
    for key in [
        "finance.einvoice.sandbox.api_url",
        "finance.einvoice.production.api_url",
        "finance.einvoice.sandbox.portal_url",
        "finance.einvoice.production.portal_url",
    ] {
        let settings = services.get::<Settings>("base/settings.store")?;
        println!("{key}: {}", settings.get_global(&**client, key).await?);
    }

    database_refusals(db).await?;
    Ok(())
}

/// The one authority this release clears documents with, as the module
/// exposes it.
fn bilibi_finance_einvoice_authority() -> impl Authority {
    Einvoice::authority()
}

/// A document for company 1 that is its lines' sum: `net` and `tax`.
fn document(code: &str, environment: &str, net: &str, tax: &str) -> String {
    format!(
        "INSERT INTO finance_einvoice_documents(tenant_id,company_id,authority,environment,document_type,code_number,issued_at,currency_code,supplier,buyer,line_extension_amount,tax_exclusive_amount,tax_amount,tax_inclusive_amount,payable_amount,issued_by_user_id,recorded_at) \
         VALUES(1,1,'myinvois','{environment}','01','{code}','2026-10-01 00:00:00','MYR','{{}}','{{}}',{net},{net},{tax},{net}+{tax},{net}+{tax},1,now());"
    )
}
// The helpers below name a sandbox document by its code number.
fn line_in(
    environment: &str,
    company: i64,
    code: &str,
    number: i32,
    net: &str,
    tax: &str,
) -> String {
    format!(
        "INSERT INTO finance_einvoice_document_lines(tenant_id,company_id,document_id,line_number,description,classification_code,quantity,unit_price,subtotal,line_extension_amount,tax_type,tax_amount) \
         SELECT 1,{company},id,{number},'Goods','022',1,{net},{net},{net},'01',{tax} FROM finance_einvoice_documents WHERE company_id=1 AND environment='{environment}' AND code_number='{code}';"
    )
}
fn line(code: &str, number: i32, net: &str, tax: &str) -> String {
    line_in("sandbox", 1, code, number, net, tax)
}
fn event(code: &str, name: &str) -> String {
    format!(
        "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,actor_type,actor_user_id,recorded_at) \
         SELECT 1,1,id,0,'{name}','user',1,now() FROM finance_einvoice_documents WHERE company_id=1 AND environment='sandbox' AND code_number='{code}';"
    )
}
fn submission(code: &str, bytes: &str) -> String {
    format!(
        "INSERT INTO finance_einvoice_submissions(tenant_id,company_id,document_id,environment,format,document_version,payload,payload_sha256,actor_type,actor_user_id,recorded_at) \
         SELECT 1,1,id,environment,'JSON','1.1','{bytes}'::bytea,encode(sha256('{bytes}'::bytea),'hex'),'user',1,now() FROM finance_einvoice_documents WHERE company_id=1 AND environment='sandbox' AND code_number='{code}';"
    )
}
fn link(code: &str, record: &str) -> String {
    format!(
        "INSERT INTO finance_einvoice_source_links(tenant_id,company_id,document_id,source_module,source_record,ledger_entry_id) \
         SELECT 1,1,id,'finance/receivables','{record}',7 FROM finance_einvoice_documents WHERE company_id=1 AND environment='sandbox' AND code_number='{code}';"
    )
}
fn source_in(source_environment: &str, consolidated: &str, covered: &str) -> String {
    format!(
        "INSERT INTO finance_einvoice_consolidated_sources(tenant_id,company_id,document_id,source_document_id,line_number) \
         SELECT 1,1,c.id,s.id,1 FROM finance_einvoice_documents c, finance_einvoice_documents s \
         WHERE c.company_id=1 AND c.environment='sandbox' AND c.code_number='{consolidated}' \
           AND s.company_id=1 AND s.environment='{source_environment}' AND s.code_number='{covered}';"
    )
}
fn source(consolidated: &str, covered: &str) -> String {
    source_in("sandbox", consolidated, covered)
}
fn tx(statements: &[String]) -> String {
    format!("BEGIN; {} COMMIT;", statements.join(" "))
}

async fn attempt(client: &tokio_postgres::Client, step: &str, sql: &str) {
    let outcome = client.batch_execute(sql).await;
    // A refusal inside an explicit BEGIN leaves the session aborted.
    let _ = client.batch_execute("ROLLBACK").await;
    match outcome {
        Ok(()) => println!("{step}: accepted"),
        Err(error) => println!(
            "{step}: refused ({})",
            error
                .as_db_error()
                .map(|e| e.message().to_owned())
                .unwrap_or_else(|| error.to_string())
        ),
    }
}

async fn database_refusals(db: &Db) -> Result<(), Box<dyn std::error::Error>> {
    let client = db.get().await?;
    println!("\n== The database's own refusals");
    // A document records with its lines in one transaction.
    attempt(
        &client,
        "INSERT INV-001 (net 100, tax 6) with its line",
        &tx(&[
            document("INV-001", "sandbox", "100", "6"),
            line("INV-001", 1, "100", "6"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT INV-002 (net 50, tax 3) with its line",
        &tx(&[
            document("INV-002", "sandbox", "50", "3"),
            line("INV-002", 1, "50", "3"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT a document with no lines",
        &tx(&[document("NOLINES", "sandbox", "10", "0")]),
    )
    .await;
    attempt(
        &client,
        "INSERT a document whose totals differ from its lines'",
        &tx(&[
            document("WRONG", "sandbox", "100", "6"),
            line("WRONG", 1, "90", "6"),
        ]),
    )
    .await;
    attempt(&client, "INSERT a document whose line amounts do not add up", &tx(&[
        document("BADLINE", "sandbox", "10", "0"),
        "INSERT INTO finance_einvoice_document_lines(tenant_id,company_id,document_id,line_number,description,classification_code,quantity,unit_price,subtotal,discount_amount,line_extension_amount,tax_type,tax_amount) \
         SELECT 1,1,id,1,'Goods','022',1,10,10,1,10,'01',0 FROM finance_einvoice_documents WHERE company_id=1 AND code_number='BADLINE';".into(),
    ])).await;
    attempt(&client, "INSERT a document whose totals do not add up", "INSERT INTO finance_einvoice_documents(tenant_id,company_id,authority,environment,document_type,code_number,issued_at,currency_code,supplier,buyer,line_extension_amount,tax_exclusive_amount,tax_amount,tax_inclusive_amount,payable_amount,issued_by_user_id,recorded_at) VALUES(1,1,'myinvois','sandbox','01','TOTALS','2026-10-01','MYR','{}','{}',100,100,6,100,100,1,now())").await;
    attempt(
        &client,
        "INSERT a second INV-001 in the sandbox",
        &tx(&[
            document("INV-001", "sandbox", "1", "0"),
            line("INV-001", 1, "1", "0"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT INV-001 in production (a separate numbering)",
        &tx(&[
            document("INV-001", "production", "1", "0"),
            line_in("production", 1, "INV-001", 1, "1", "0"),
        ]),
    )
    .await;
    attempt(&client, "INSERT a document with a lowercase currency", "INSERT INTO finance_einvoice_documents(tenant_id,company_id,authority,environment,document_type,code_number,issued_at,currency_code,supplier,buyer,line_extension_amount,tax_exclusive_amount,tax_amount,tax_inclusive_amount,payable_amount,issued_by_user_id,recorded_at) VALUES(1,1,'myinvois','sandbox','01','CUR','2026-10-01','myr','{}','{}',0,0,0,0,0,1,now())").await;
    attempt(&client, "INSERT a note that is its own original", "INSERT INTO finance_einvoice_documents(id,tenant_id,company_id,authority,environment,document_type,code_number,issued_at,currency_code,supplier,buyer,original_document_id,line_extension_amount,tax_exclusive_amount,tax_amount,tax_inclusive_amount,payable_amount,issued_by_user_id,recorded_at) VALUES(900,1,1,'myinvois','sandbox','02','SELF','2026-10-01','MYR','{}','{}',900,0,0,0,0,0,1,now())").await;
    attempt(
        &client,
        "INSERT a line into the recorded INV-001",
        &line("INV-001", 2, "1", "0"),
    )
    .await;
    attempt(
        &client,
        "UPDATE a document",
        "UPDATE finance_einvoice_documents SET code_number='X' WHERE code_number='INV-001'",
    )
    .await;
    attempt(
        &client,
        "DELETE a document",
        "DELETE FROM finance_einvoice_documents WHERE code_number='INV-001'",
    )
    .await;
    attempt(
        &client,
        "UPDATE a line",
        "UPDATE finance_einvoice_document_lines SET quantity=2",
    )
    .await;
    attempt(
        &client,
        "DELETE a line",
        "DELETE FROM finance_einvoice_document_lines",
    )
    .await;
    attempt(
        &client,
        "TRUNCATE the documents",
        "TRUNCATE finance_einvoice_documents",
    )
    .await;
    attempt(
        &client,
        "TRUNCATE the lines",
        "TRUNCATE finance_einvoice_document_lines",
    )
    .await;

    println!("-- Status events are numbered by the database");
    attempt(
        &client,
        "INSERT issued and submitted events for INV-001",
        &tx(&[event("INV-001", "issued"), event("INV-001", "submitted")]),
    )
    .await;
    let numbered: Vec<String> = client
        .query("SELECT sequence::text||' '||event FROM finance_einvoice_status_events e JOIN finance_einvoice_documents d ON d.id=e.document_id WHERE d.code_number='INV-001' ORDER BY sequence", &[])
        .await?
        .iter()
        .map(|row| row.get(0))
        .collect();
    println!("INV-001's events: {numbered:?}");
    attempt(
        &client,
        "INSERT an event with a capitalised name",
        &event("INV-001", "Valid"),
    )
    .await;
    attempt(&client, "INSERT an event by a user without an id", "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,actor_type,recorded_at) SELECT 1,1,id,0,'valid','user',now() FROM finance_einvoice_documents WHERE code_number='INV-001'").await;
    attempt(&client, "INSERT an event with an HTTP status of 99", "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,http_status,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,0,'valid',99,'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-001'").await;
    attempt(&client, "INSERT a system event with a principal", "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,actor_type,system_principal,recorded_at) SELECT 1,1,id,0,'polled','system','finance.einvoice',now() FROM finance_einvoice_documents WHERE code_number='INV-001'").await;
    attempt(&client, "INSERT a first posted event", &format!("INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,ledger_entry_id,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,0,'posted',5,'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002';")).await;
    attempt(&client, "INSERT a second posted event", &format!("INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,ledger_entry_id,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,0,'posted',6,'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002';")).await;
    attempt(&client, "INSERT an event with a ledger entry of 0", "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,ledger_entry_id,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,0,'posting_reversed',0,'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002'").await;
    attempt(&client, "INSERT a first and a second posting_reversed event", &tx(&[
        "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,ledger_entry_id,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,0,'posting_reversed',8,'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002';".into(),
        "INSERT INTO finance_einvoice_status_events(tenant_id,company_id,document_id,sequence,event,ledger_entry_id,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,0,'posting_reversed',9,'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002';".into(),
    ])).await;
    attempt(
        &client,
        "UPDATE an event",
        "UPDATE finance_einvoice_status_events SET event='valid'",
    )
    .await;
    attempt(
        &client,
        "DELETE an event",
        "DELETE FROM finance_einvoice_status_events",
    )
    .await;
    attempt(
        &client,
        "TRUNCATE the events",
        "TRUNCATE finance_einvoice_status_events",
    )
    .await;

    println!("-- Submissions keep the bytes they hash");
    attempt(
        &client,
        "INSERT a submission whose hash matches its bytes",
        &submission("INV-002", "{\"Invoice\":[]}"),
    )
    .await;
    attempt(&client, "INSERT a submission whose hash does not match", "INSERT INTO finance_einvoice_submissions(tenant_id,company_id,document_id,environment,format,document_version,payload,payload_sha256,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,environment,'JSON','1.1','abc'::bytea,repeat('0',64),'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002'").await;
    attempt(&client, "INSERT a submission with no bytes", "INSERT INTO finance_einvoice_submissions(tenant_id,company_id,document_id,environment,format,document_version,payload,payload_sha256,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,environment,'JSON','1.1',''::bytea,encode(sha256(''::bytea),'hex'),'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002'").await;
    attempt(&client, "INSERT a submission in a format the authority lacks", "INSERT INTO finance_einvoice_submissions(tenant_id,company_id,document_id,environment,format,document_version,payload,payload_sha256,actor_type,actor_user_id,recorded_at) SELECT 1,1,id,environment,'CSV','1.1','abc'::bytea,encode(sha256('abc'::bytea),'hex'),'user',1,now() FROM finance_einvoice_documents WHERE code_number='INV-002'").await;
    attempt(
        &client,
        "UPDATE a submission",
        "UPDATE finance_einvoice_submissions SET format='XML'",
    )
    .await;
    attempt(
        &client,
        "DELETE a submission",
        "DELETE FROM finance_einvoice_submissions",
    )
    .await;
    attempt(
        &client,
        "TRUNCATE the submissions",
        "TRUNCATE finance_einvoice_submissions",
    )
    .await;

    println!("-- A consolidated document covers sources once");
    attempt(&client, "INSERT CONS-1 (net 150, tax 9) covering INV-001 and INV-002 (INV-002 was submitted on its own)", &tx(&[
        document("CONS-1", "sandbox", "150", "9"), line("CONS-1", 1, "150", "9"),
        source("CONS-1", "INV-001"), source("CONS-1", "INV-002"),
    ])).await;
    attempt(
        &client,
        "INSERT SRC-1 and SRC-2 (nets 100 and 50)",
        &tx(&[
            document("SRC-1", "sandbox", "100", "6"),
            line("SRC-1", 1, "100", "6"),
            document("SRC-2", "sandbox", "50", "3"),
            line("SRC-2", 1, "50", "3"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT CONS-2 (net 140) covering SRC-1 and SRC-2 (their sum is 150)",
        &tx(&[
            document("CONS-2", "sandbox", "140", "9"),
            line("CONS-2", 1, "140", "9"),
            source("CONS-2", "SRC-1"),
            source("CONS-2", "SRC-2"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT CONS-3 (net 150, tax 9) covering SRC-1 and SRC-2",
        &tx(&[
            document("CONS-3", "sandbox", "150", "9"),
            line("CONS-3", 1, "150", "9"),
            source("CONS-3", "SRC-1"),
            source("CONS-3", "SRC-2"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT CONS-4 covering SRC-1 again",
        &tx(&[
            document("CONS-4", "sandbox", "100", "6"),
            line("CONS-4", 1, "100", "6"),
            source("CONS-4", "SRC-1"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT CONS-5 covering the production INV-001 from the sandbox",
        &tx(&[
            document("CONS-5", "sandbox", "1", "0"),
            line("CONS-5", 1, "1", "0"),
            source_in("production", "CONS-5", "INV-001"),
        ]),
    )
    .await;
    attempt(
        &client,
        "SUBMIT SRC-1 on its own while CONS-3 holds it",
        &submission("SRC-1", "abc"),
    )
    .await;
    attempt(&client, "SUBMIT CONS-3", &submission("CONS-3", "abc")).await;
    attempt(
        &client,
        "INSERT a source that is the consolidated document itself",
        &tx(&[
            document("CONS-6", "sandbox", "1", "0"),
            line("CONS-6", 1, "1", "0"),
            source("CONS-6", "CONS-6"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT FRESH-1",
        &tx(&[
            document("FRESH-1", "sandbox", "1", "0"),
            line("FRESH-1", 1, "1", "0"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT a source row into recorded CONS-3",
        &source("CONS-3", "FRESH-1"),
    )
    .await;
    attempt(
        &client,
        "CANCEL CONS-3 (releases SRC-1 and SRC-2)",
        &event("CONS-3", "cancelled"),
    )
    .await;
    attempt(
        &client,
        "SUBMIT CONS-3 again after it released its sources",
        &submission("CONS-3", "abc"),
    )
    .await;
    attempt(
        &client,
        "INSERT CONS-7 covering the released SRC-1",
        &tx(&[
            document("CONS-7", "sandbox", "100", "6"),
            line("CONS-7", 1, "100", "6"),
            source("CONS-7", "SRC-1"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT CONS-8 covering CONS-7 (a consolidated document is never a source)",
        &tx(&[
            document("CONS-8", "sandbox", "100", "6"),
            line("CONS-8", 1, "100", "6"),
            source("CONS-8", "CONS-7"),
        ]),
    )
    .await;
    attempt(
        &client,
        "UPDATE a source row",
        "UPDATE finance_einvoice_consolidated_sources SET line_number=1",
    )
    .await;
    attempt(
        &client,
        "DELETE a source row",
        "DELETE FROM finance_einvoice_consolidated_sources",
    )
    .await;
    attempt(
        &client,
        "TRUNCATE the source rows",
        "TRUNCATE finance_einvoice_consolidated_sources",
    )
    .await;

    println!("-- A source transaction has one standing document");
    attempt(
        &client,
        "INSERT LNK-1 and LNK-2 (issued), both linking receivables invoice:17",
        &tx(&[
            document("LNK-1", "sandbox", "10", "0"),
            line("LNK-1", 1, "10", "0"),
            link("LNK-1", "invoice:17"),
            event("LNK-1", "issued"),
            document("LNK-2", "sandbox", "10", "0"),
            line("LNK-2", 1, "10", "0"),
            link("LNK-2", "invoice:17"),
            event("LNK-2", "issued"),
        ]),
    )
    .await;
    attempt(
        &client,
        "INSERT LNK-3 linking invoice:17 and invoice:17 twice",
        &tx(&[
            document("LNK-3", "sandbox", "10", "0"),
            line("LNK-3", 1, "10", "0"),
            link("LNK-3", "invoice:17"),
            link("LNK-3", "invoice:17"),
        ]),
    )
    .await;
    attempt(&client, "INSERT LNK-4 with a source module of the wrong shape", &tx(&[
        document("LNK-4", "sandbox", "10", "0"), line("LNK-4", 1, "10", "0"),
        "INSERT INTO finance_einvoice_source_links(tenant_id,company_id,document_id,source_module,source_record,ledger_entry_id) SELECT 1,1,id,'Receivables','invoice:18',7 FROM finance_einvoice_documents WHERE company_id=1 AND code_number='LNK-4';".into(),
    ])).await;
    attempt(
        &client,
        "INSERT LNK-5 with a source record of the wrong shape",
        &tx(&[
            document("LNK-5", "sandbox", "10", "0"),
            line("LNK-5", 1, "10", "0"),
            link("LNK-5", "invoice:0"),
        ]),
    )
    .await;
    attempt(
        &client,
        "SUBMIT LNK-1 (LNK-2 does not stand yet)",
        &submission("LNK-1", "abc"),
    )
    .await;
    attempt(
        &client,
        "SUBMIT LNK-2 (LNK-1 has a submission no answer followed)",
        &submission("LNK-2", "abc"),
    )
    .await;
    attempt(
        &client,
        "INSERT LNK-6 linking invoice:17 while LNK-1 stands",
        &tx(&[
            document("LNK-6", "sandbox", "10", "0"),
            line("LNK-6", 1, "10", "0"),
            link("LNK-6", "invoice:17"),
        ]),
    )
    .await;
    attempt(
        &client,
        "REFUSE LNK-1 (it no longer stands)",
        &event("LNK-1", "refused"),
    )
    .await;
    attempt(
        &client,
        "SUBMIT LNK-2 once LNK-1 was refused",
        &submission("LNK-2", "abc"),
    )
    .await;
    attempt(
        &client,
        "INSERT a link into recorded LNK-2",
        &link("LNK-2", "invoice:19"),
    )
    .await;
    attempt(
        &client,
        "UPDATE a link",
        "UPDATE finance_einvoice_source_links SET ledger_entry_id=8",
    )
    .await;
    attempt(
        &client,
        "DELETE a link",
        "DELETE FROM finance_einvoice_source_links",
    )
    .await;
    attempt(
        &client,
        "TRUNCATE the links",
        "TRUNCATE finance_einvoice_source_links",
    )
    .await;

    println!("-- Rows of one company reference only that company");
    attempt(
        &client,
        "INSERT a line for company 2 under company 1's document",
        &tx(&[
            document("OTHER-1", "sandbox", "1", "0"),
            line_in("sandbox", 2, "OTHER-1", 1, "1", "0"),
        ]),
    )
    .await;
    let triggers: i64 = client
        .query_one("SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid WHERE c.relname LIKE 'finance_einvoice_%' AND NOT t.tgisinternal", &[])
        .await?
        .get(0);
    let functions: i64 = client
        .query_one(
            "SELECT count(*) FROM pg_proc WHERE proname LIKE 'finance_einvoice_%'",
            &[],
        )
        .await?
        .get(0);
    println!("triggers on the e-invoice tables: {triggers}; functions: {functions}");
    Ok(())
}
