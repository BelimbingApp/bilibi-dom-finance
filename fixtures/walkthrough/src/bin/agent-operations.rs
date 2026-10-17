//! Provision real OAuth credentials for the HTTP/MCP ledger walkthrough on
//! an explicitly disposable database. No book is opened here: the wire proof
//! starts from an empty ledger. Never print credentials or plan codes.
use bilibi_authz::grants::Grants;
use bilibi_core::agent_access::{
    code_challenge, AgentAccess, AuthorizeParams, CodeExchange, Decision, CLIENT_ID, OAUTH_CLIENTS,
};
use bilibi_database::{pool, Error, MutationBoundary};
use bilibi_finance_walkthrough_fixture::sign_in;
use bilibi_module_registry::Services;
use bilibi_settings::{Settings, Target};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::Arc,
    time::{Duration, SystemTime},
};

bilibi_walkthrough_fixture::walkthrough_vocabulary!();
bilibi_walkthrough_fixture::walkthrough_module_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_ledger_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_attachments_roles!();
bilibi_finance_walkthrough_fixture::walkthrough_einvoice_roles!();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("BILIBI_AGENT_PROOF").as_deref() != Ok("disposable") {
        return Err(Error::Invalid.into());
    }
    let path = std::env::var("BILIBI_AGENT_CREDENTIALS")?;
    let origin = std::env::var("APP_ORIGIN")?;
    bilibi_base::link();
    bilibi_core::link();
    bilibi_domains::link();
    bilibi_extensions::link();
    let db = pool(&std::env::var("DATABASE_URL")?)?;
    let modules = bilibi_module_registry::discover()?;
    bilibi_module_registry::verify_composition(
        &modules,
        bilibi_domains::MODULES,
        bilibi_extensions::MODULES,
    )?;
    bilibi_database::migrate(&db, &modules).await?;
    bilibi_walkthrough_fixture::install(&db).await?;
    let mut services = Services::default();
    services.provide("bootstrap.database", Arc::new(db.clone()))?;
    services.install(&modules)?;
    let grants = services.get::<Grants>("base/authz.grants")?;
    grants.reconcile().await?;
    let admin = sign_in(&db, &services, 1, 4).await?;
    // A second real Person/Company affiliation, not an actor passed to a write.
    for key in bilibi_finance_ledger::CAPABILITIES
        .into_iter()
        .chain(["admin.company.tenant-wide.manage"])
    {
        grants.set_direct(&admin, 7, key, Some(true)).await?;
    }
    // H3: both real people can exercise Payables; this changes only the
    // explicitly disposable walkthrough database, never production seeds.
    for user in [1, 7] {
        for key in bilibi_finance_payables::CAPABILITIES {
            grants.set_direct(&admin, user, key, Some(true)).await?;
        }
    }
    let manager = sign_in(&db, &services, 1, 1).await?;
    let settings = services.get::<Settings>("base/settings.store")?;
    let boundary = services.get::<MutationBoundary>("base/database.mutations")?;
    let mut client = db.get().await?;
    let mut tx = boundary
        .begin(
            &mut client,
            admin.mutation_context("walkthrough.agent.clients"),
        )
        .await?;
    let callback = "https://example.test/ledger-proof/callback";
    settings
        .put(
            &mut tx,
            &admin,
            Target::Global,
            OAUTH_CLIENTS,
            json!({"ledger-proof":{"name":"Ledger walkthrough","redirect_uris":[callback]}}),
        )
        .await?;
    // Bound only this disposable proof, not production defaults.
    for key in [
        "agent_api.mcp_requests_per_minute",
        "agent_access.oauth_requests_per_minute",
    ] {
        settings
            .put(&mut tx, &admin, Target::Global, key, json!(10000))
            .await?;
    }
    tx.commit().await?;
    drop(client);
    let access = services.get::<AgentAccess>("core/agent_access.grants")?;
    let resource = format!("{}/mcp", origin.trim_end_matches('/'));
    let mut credentials = serde_json::Map::new();
    for (name, scope) in [
        ("manager", manager),
        ("approver", sign_in(&db, &services, 1, 7).await?),
        ("other", sign_in(&db, &services, 2, 5).await?),
    ] {
        let expiry = SystemTime::now() + Duration::from_secs(3600);
        let device = access.start_device(CLIENT_ID, "ledger-walkthrough").await?;
        access
            .decide(&scope, &device.user_code, Decision::Approve, expiry)
            .await?;
        let api = access.exchange(CLIENT_ID, &device.device_code).await?;
        let verifier = "ledger-walkthrough-verifier-with-at-least-forty-three-characters";
        let params = AuthorizeParams {
            client_id: "ledger-proof".into(),
            redirect_uri: callback.into(),
            response_type: "code".into(),
            resource: resource.clone(),
            scope: Some("bilibi".into()),
            code_challenge: code_challenge(verifier),
            code_challenge_method: "S256".into(),
            state: Some("ledger-walkthrough".into()),
        };
        let code = access.authorize(&scope, &params, &resource, expiry).await?;
        let mcp = access
            .exchange_code(
                &CodeExchange {
                    code,
                    client_id: params.client_id,
                    redirect_uri: params.redirect_uri,
                    code_verifier: verifier.into(),
                    resource: resource.clone(),
                },
                &resource,
            )
            .await?;
        credentials.insert(
            name.into(),
            json!({"api":api.access_token,"mcp":mcp.access_token}),
        );
    }
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(serde_json::to_string(&credentials)?.as_bytes())?;
    println!("Disposable fixture ready: empty ledgers; real manager, independent approver and other-tenant OAuth credentials saved privately.");
    Ok(())
}
