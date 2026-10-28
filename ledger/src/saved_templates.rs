//! Reusable company/practice charts: explicit publication of an immutable
//! tenant-shared snapshot. The source company remains the audit owner, not a
//! live dependency of a company's applied chart.
use crate::{
    accounts, configuration::DEFAULT_TEMPLATE, templates, ChartTemplate, ChartTemplateRow,
    ChartTemplateSection, FieldError, Ledger, LedgerError, Result, MANAGE_TEMPLATES, VIEW,
};
use bilibi_agent_api::OperationClient;
use bilibi_database::{Change, Error, MutationTransaction};
use bilibi_settings::{Settings, Target};
use bilibi_tenancy::Scope;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
};

static SETTINGS: OnceLock<Arc<Settings>> = OnceLock::new();
pub(crate) fn bind(settings: Arc<Settings>) -> bilibi_module_registry::Result<()> {
    SETTINGS.set(settings).map_err(|_| {
        bilibi_module_registry::RegistryError::Service("duplicate ledger template binding".into())
    })
}
fn settings() -> Result<&'static Settings> {
    SETTINGS
        .get()
        .map(Arc::as_ref)
        .ok_or(Error::Internal.into())
}
fn invalid(field: &'static str, message: &str) -> LedgerError {
    LedgerError::Invalid(vec![FieldError::new(field, message)])
}

/// Publish the current company's active accounts and groups. Each optional
/// group and all its descendants becomes one optional section. Nested section
/// roots are refused; nothing is copied from postings or module settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewChartTemplate {
    /// A stable lowercase slug, without the reserved `saved:` prefix.
    pub key: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub optional_group_codes: Vec<String>,
}

pub(crate) async fn all(client: &dyn OperationClient, tenant: i64) -> Result<Vec<ChartTemplate>> {
    let mut all = templates::all().to_vec();
    for row in client
        .query(
            "SELECT template FROM finance_ledger_chart_templates WHERE tenant_id=$1 ORDER BY key",
            &[&tenant],
        )
        .await?
    {
        all.push(serde_json::from_value(row.get(0)).map_err(|_| Error::Internal)?);
    }
    Ok(all)
}
pub(crate) async fn require(
    client: &dyn OperationClient,
    tenant: i64,
    key: &str,
) -> Result<ChartTemplate> {
    if let Some(template) = templates::find(key) {
        return Ok(template.clone());
    }
    let row = client
        .query_opt(
            "SELECT template FROM finance_ledger_chart_templates WHERE tenant_id=$1 AND key=$2",
            &[&tenant, &key],
        )
        .await?;
    row.map(|r| serde_json::from_value(r.get(0)).map_err(|_| LedgerError::Failed(Error::Internal)))
        .transpose()?
        .ok_or_else(|| invalid("template", "is not a starter chart"))
}
pub(crate) async fn proposal(client: &dyn OperationClient, scope: &Scope) -> Result<Value> {
    let key = settings()?
        .get(client, scope, Target::Tenant, DEFAULT_TEMPLATE)
        .await?
        .value;
    let proposed = match key.as_str() {
        Some(key) => all(client, scope.tenant())
            .await?
            .into_iter()
            .find(|t| t.key == key),
        None => None,
    };
    Ok(json!({"configured":key,"proposed":proposed}))
}

pub(crate) async fn prepare(
    client: &dyn OperationClient,
    scope: &Scope,
    company: i64,
    new: &NewChartTemplate,
) -> Result<ChartTemplate> {
    crate::agent_operations::authorize_on(client, scope, company, MANAGE_TEMPLATES).await?;
    crate::agent_operations::authorize_on(client, scope, company, VIEW).await?;
    if new.key.len() > 64
        || new.key.is_empty()
        || new.key.split('-').any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
    {
        return Err(invalid(
            "key",
            "must be a lowercase slug of at most 64 characters",
        ));
    }
    if new.name.trim().is_empty() || new.name.trim().chars().count() > 200 {
        return Err(invalid("name", "must contain between 1 and 200 characters"));
    }
    if new.description.trim().is_empty() || new.description.trim().chars().count() > 1000 {
        return Err(invalid(
            "description",
            "must contain between 1 and 1000 characters",
        ));
    }
    let key = format!("saved:{}", new.key);
    if client
        .query_opt(
            "SELECT id FROM finance_ledger_chart_templates WHERE tenant_id=$1 AND key=$2",
            &[&scope.tenant(), &key],
        )
        .await?
        .is_some()
    {
        return Err(invalid(
            "key",
            "has already been taken; save a new template with another key",
        ));
    }
    let mut chart: Vec<_> = accounts::list(client, scope.tenant(), company)
        .await?
        .into_iter()
        .filter(|a| a.active)
        .collect();
    if chart.is_empty() || chart.len() > 2000 {
        return Err(invalid(
            "base",
            "Save a chart with between 1 and 2000 active accounts and groups",
        ));
    }
    let optional: BTreeSet<_> = new.optional_group_codes.iter().collect();
    if optional
        .iter()
        .any(|code| !chart.iter().any(|a| a.is_group && &a.code == *code))
    {
        return Err(invalid(
            "optional_group_codes",
            "must name groups in the current company's chart",
        ));
    }
    let mut made: BTreeMap<i64, (String, Option<String>)> = BTreeMap::new();
    let mut rows = Vec::new();
    let mut sections = Vec::new();
    while !chart.is_empty() {
        let Some(index) = chart
            .iter()
            .position(|a| a.parent_id.is_none_or(|id| made.contains_key(&id)))
        else {
            return Err(invalid("base", "The chart has an invalid group hierarchy"));
        };
        let account = chart.remove(index);
        let parent = account.parent_id.and_then(|id| made.get(&id));
        let inherited = parent.and_then(|(_, section)| section.clone());
        let section = if optional.contains(&account.code) {
            if inherited.is_some() {
                return Err(invalid(
                    "optional_group_codes",
                    "cannot include a group inside another optional group",
                ));
            }
            sections.push(ChartTemplateSection {
                key: format!("group-{}", account.id),
                name: account.name.clone(),
                description: format!("Include {} and its accounts.", account.name),
            });
            Some(format!("group-{}", account.id))
        } else {
            inherited
        };
        rows.push(ChartTemplateRow {
            code: account.code.clone(),
            name: account.name,
            account_type: account.account_type,
            is_group: account.is_group,
            parent_code: parent.map(|(code, _)| code.clone()),
            retained_earnings: account.retained_earnings,
            section: section.clone(),
        });
        made.insert(account.id, (account.code, section));
    }
    if rows.iter().all(|row| row.section.is_some()) {
        return Err(invalid(
            "optional_group_codes",
            "must leave at least one account or group in the core chart",
        ));
    }
    Ok(ChartTemplate { key, name: new.name.trim().into(), description: new.description.trim().into(),
        review: "Company chart snapshot; review before use".into(),
        notes: vec!["Shared within this tenant. Active accounts and groups only; no balances, currency, report layout or subledger settings. Later changes to the source chart do not change this template.".into()], sections, rows })
}

impl Ledger {
    /// Authorized discovery of the configured proposal. Never applies it.
    pub async fn chart_template_proposal(&self, scope: &Scope, company: i64) -> Result<Value> {
        let client = self.db.get().await.map_err(Error::from)?;
        crate::authorize(&**client, scope, company, VIEW).await?;
        proposal(&**client, scope).await
    }
    /// Publish an immutable tenant-shared chart snapshot, explicitly reviewed
    /// with `finance.ledger.templates.manage`, independently of account editing.
    pub async fn save_chart_template(
        &self,
        scope: &Scope,
        company: i64,
        new: &NewChartTemplate,
    ) -> Result<ChartTemplate> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company,
                &[MANAGE_TEMPLATES, VIEW],
                "finance.ledger.chart_template.save",
            )
            .await?;
        let saved = Self::save_chart_template_on(&mut tx, scope, company, new).await?;
        tx.commit().await?;
        Ok(saved)
    }
    pub(super) async fn save_chart_template_on(
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        new: &NewChartTemplate,
    ) -> Result<ChartTemplate> {
        crate::join(tx, scope, company, MANAGE_TEMPLATES).await?;
        // Different source companies can publish the same key concurrently.
        // Serialize the refusal before entering the audited mutation closure.
        tx.sql()
            .execute(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
                &[&format!("finance-template:{}:{}", scope.tenant(), new.key)],
            )
            .await
            .map_err(Error::from)?;
        let template = prepare(tx.sql(), scope, company, new).await?;
        let tenant = scope.tenant();
        let saved = template.clone();
        tx.mutate(move |sql| Box::pin(async move {
            let row = sql.query_one("INSERT INTO finance_ledger_chart_templates(tenant_id,company_id,key,template) VALUES($1,$2,$3,$4) RETURNING id,inserted_at::text", &[&tenant, &company, &template.key, &json!(template)]).await?;
            let id: i64 = row.get(0);
            Ok(((), vec![Change { kind: "finance/ledger.chart_template".into(), id, old: Value::Null,
                new: json!({"id":id,"tenant_id":tenant,"company_id":company,"key":template.key,"template":template,"inserted_at":row.get::<_,String>(1)}) }]))
        })).await?;
        Ok(saved)
    }
    /// Set or clear this tenant's proposal. Existing books are never changed.
    pub async fn propose_chart_template(
        &self,
        scope: &Scope,
        company: i64,
        key: Option<&str>,
    ) -> Result<Value> {
        let mut client = self.db.get().await.map_err(Error::from)?;
        let mut tx = self
            .write(
                &mut client,
                scope,
                company,
                &[MANAGE_TEMPLATES],
                "finance.ledger.chart_template.propose",
            )
            .await?;
        let value = Self::propose_chart_template_on(&mut tx, scope, company, key).await?;
        tx.commit().await?;
        Ok(value)
    }
    pub(super) async fn propose_chart_template_on(
        tx: &mut MutationTransaction<'_>,
        scope: &Scope,
        company: i64,
        key: Option<&str>,
    ) -> Result<Value> {
        crate::join(tx, scope, company, MANAGE_TEMPLATES).await?;
        if let Some(key) = key {
            require(tx.sql(), scope.tenant(), key).await?;
        }
        settings()?
            .put(tx, scope, Target::Tenant, DEFAULT_TEMPLATE, json!(key))
            .await?;
        proposal(tx.sql(), scope).await
    }
}
