//! Bilimbi `Finance.Reconciliation.Web`: bank accounts and statement import.
mod account;
mod format;
mod index;

use crate::{Reconciliation, MODULE};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_datetime::{DisplayContext, DisplayPolicy};
use bilibi_finance_attachments::Attachments;
use bilibi_finance_ledger::Ledger;
use bilibi_locale::Locales;
use bilibi_module_registry::{
    contributions::{Access, Contribution, Method, Route, ROUTES},
    Provenance,
};
use bilibi_tenancy::Scope;
use bilibi_ui::{page::*, Html};
use std::sync::Arc;

pub(crate) const PAGES: &str = "finance/reconciliation.pages";
const DENIED: &str = "You do not have permission for that action.";

#[derive(Clone)]
pub(crate) struct Pages {
    pub db: Db,
    pub reconciliation: Arc<Reconciliation>,
    pub ledger: Arc<Ledger>,
    pub attachments: Arc<Attachments>,
    pub display: Arc<DisplayPolicy>,
    pub locales: Arc<Locales>,
}

bilibi_menu::contribute!(
    "finance/reconciliation",
    bilibi_menu::MenuItem::page(
        "finance.reconciliation",
        "Bank reconciliation",
        "/finance/reconciliation"
    )
    .icon("building-library")
    .under("finance")
    .requires(bilibi_menu::Requirement::Capability(crate::VIEW))
    .order(15)
);

macro_rules! route {
    ($id:literal, $path:literal, $op:literal) => {
        inventory::submit! { Contribution::new(ROUTES, $id,
        Provenance { module: MODULE, source: file!(), line: line!() },
        &Route { method: Method::Get, path: $path, service: PAGES, operation: $op,
            access: Access::Capability(crate::VIEW), framed: true }) }
    };
}
route!(
    "finance/reconciliation.index",
    "/finance/reconciliation",
    "index"
);
route!(
    "finance/reconciliation.account",
    "/finance/reconciliation/accounts/{id}",
    "account"
);

fn company(scope: &Scope) -> Result<i64> {
    scope
        .actor()
        .organisation()
        .map(|id| id.get())
        .ok_or(Error::NotFound)
}
fn render(template: impl Template) -> Result<Html> {
    Html::render(&template).map_err(|_| Error::Internal)
}
impl Pages {
    async fn allowed(&self, scope: &Scope, capability: &str) -> Result<bool> {
        let client = self.db.get().await?;
        match bilibi_company::organisation::authorize_target(
            &**client,
            scope,
            company(scope)?,
            capability,
        )
        .await
        {
            Ok(_) => Ok(true),
            Err(Error::Forbidden | Error::Unauthorized | Error::NotFound) => Ok(false),
            Err(e) => Err(e),
        }
    }
    async fn display(&self, scope: &Scope) -> Result<DisplayContext> {
        let client = self.db.get().await?;
        let locale = self.locales.resolve(&**client, scope, None).await?.locale;
        self.display.display(&**client, scope, None, locale).await
    }
    fn state(&self, request: &PageRequest<'_, Scope>) -> Result<Box<dyn LiveState<Scope, Error>>> {
        match request.operation {
            "index" => Ok(Box::new(index::Index::new(
                self,
                request.query,
                request.framed,
            )?)),
            "account" => {
                let id = request
                    .path
                    .get("id")
                    .and_then(|s| s.parse::<i64>().ok())
                    .filter(|id| *id > 0)
                    .ok_or(Error::NotFound)?;
                Ok(Box::new(account::Account::new(
                    self,
                    id,
                    request.query,
                    request.framed,
                )?))
            }
            _ => Err(Error::NotFound),
        }
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "index" => Some("Bank reconciliation"),
            "account" => Some("Bank statements"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            let mut state = self.state(&request)?;
            let body = state
                .update(LiveRequest {
                    scope,
                    credential: "",
                    input: LiveInput::Refresh,
                })
                .await?;
            let title = if request.operation == "account" {
                let id = request
                    .path
                    .get("id")
                    .and_then(|s| s.parse().ok())
                    .ok_or(Error::NotFound)?;
                format::told(
                    self.reconciliation
                        .get_bank_account(scope, company(scope)?, id)
                        .await,
                )?
                .ok()
                .map(|account| account.name)
            } else {
                None
            };
            Ok(PageBody {
                html: body.html,
                live: true,
                title,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move { Ok(Some(self.state(&request)?)) })
    }
}
fn event_type(value: &serde_json::Value) -> Result<&str> {
    value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(Error::Invalid)
}
fn record_id(value: &serde_json::Value) -> Option<i64> {
    value
        .get("id")
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse().ok())
                .or_else(|| v.as_i64())
        })
        .filter(|id| *id > 0)
}
fn flash_map(flash: &Option<(bilibi_ui::StatusKind, String)>) -> bilibi_ui::FlashMap<'_> {
    let mut map = bilibi_ui::NO_FLASH;
    if let Some((kind, text)) = flash {
        match kind {
            bilibi_ui::StatusKind::Success => map.success = text,
            bilibi_ui::StatusKind::Error => map.error = text,
            bilibi_ui::StatusKind::Info => map.info = text,
            bilibi_ui::StatusKind::Warning => map.warning = text,
        }
    }
    map
}
