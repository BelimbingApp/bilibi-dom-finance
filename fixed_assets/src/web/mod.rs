//! Bilimbi Finance.FixedAssets.Web: shared lists and the asset record page.
mod detail;
mod index;
mod input;

use crate::{FixedAssets, FixedAssetsError, MODULE};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_datetime::{DisplayContext, DisplayPolicy};
use bilibi_finance_ledger::Ledger;
use bilibi_locale::Locales;
use bilibi_module_registry::{
    contributions::{Access, Method, Route, ROUTES},
    Contribution, Provenance,
};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::*, params, BuiltinColumn, Cell, EmptyState, Field, FlexRow, FlexTable, Html, Instant,
    ListColumns, StatusKind, TableEmpty,
};
use std::{collections::HashMap, sync::Arc};

pub(crate) const PAGES: &str = "finance/fixed_assets.pages";
const DENIED: &str = "You do not have permission for that action.";
#[derive(Clone)]
pub(crate) struct Pages {
    pub db: Db,
    pub assets: Arc<FixedAssets>,
    pub ledger: Arc<Ledger>,
    pub display: Arc<DisplayPolicy>,
    pub locales: Arc<Locales>,
}
macro_rules! route {
    ($op:literal, $path:literal) => {
        inventory::submit! { Contribution::new(ROUTES, concat!("finance/fixed_assets.", $op), Provenance { module: MODULE, source: file!(), line: line!() }, &Route {
            method: Method::Get, path: $path, service: PAGES, operation: $op,
            access: Access::Capability(crate::VIEW), framed: true,
        }) }
    };
}
route!("register", "/finance/fixed-assets/register");
route!("categories", "/finance/fixed-assets/categories");
route!("runs", "/finance/fixed-assets/runs");
route!("asset", "/finance/fixed-assets/register/{id}");
macro_rules! menu {
    ($kind:literal, $label:literal, $order:literal) => {
        bilibi_menu::contribute!(
            "finance/fixed_assets",
            bilibi_menu::MenuItem::page(
                concat!("finance.fixed-assets.", $kind),
                $label,
                concat!("/finance/fixed-assets/", $kind)
            )
            .icon("clipboard-document-list")
            .under("finance")
            .requires(bilibi_menu::Requirement::Capability(crate::VIEW))
            .order($order)
        );
    };
}
menu!("register", "Fixed assets", 40);
menu!("categories", "Asset categories", 41);
menu!("runs", "Depreciation runs", 42);

fn render(t: impl Template) -> Result<Html> {
    Html::render(&t).map_err(|_| Error::Internal)
}
fn company(s: &Scope) -> Option<i64> {
    s.actor().organisation().map(|p| p.get())
}
fn refusal(e: FixedAssetsError) -> String {
    use bilibi_finance_ledger::LedgerError;
    match e {
        FixedAssetsError::Invalid(fields)
        | FixedAssetsError::Ledger(LedgerError::Invalid(fields)) => fields
            .iter()
            .map(|f| format!("{}: {}", f.field, f.message))
            .collect::<Vec<_>>()
            .join("; "),
        FixedAssetsError::Failed(_) | FixedAssetsError::Ledger(LedgerError::Failed(_)) => {
            "The operation was refused. Refresh this page and review the record.".into()
        }
        e => e.to_string(),
    }
}
fn cols(id: &str, labels: &[(&str, &str)]) -> Result<ListColumns> {
    ListColumns::mount(
        id,
        labels
            .iter()
            .map(|(key, label)| BuiltinColumn::new(key, label))
            .collect(),
    )
    .map_err(|_| Error::Internal)
}
fn table(
    id: &str,
    columns: &ListColumns,
    rows: Vec<FlexRow>,
    caption: &str,
    event: &'static str,
    empty: &str,
) -> Result<Html> {
    let mut t = FlexTable::new(id, columns.flex_columns(), rows);
    t.event = event;
    t.zoom = columns.zoom;
    t.suggestions = columns.flex_suggestions();
    t.add_query = &columns.add_query;
    t.caption = caption;
    t.framed = true;
    t.empty = Some(TableEmpty::Content(Html::text(empty)));
    render(t)
}
fn time_cell(display: &DisplayContext, value: &str) -> Result<Cell> {
    let at = value
        .parse::<jiff::Timestamp>()
        .map_err(|_| Error::Invalid)?;
    let r = display.render(
        at,
        bilibi_datetime::Format::DateTime,
        bilibi_datetime::Precision::Minute,
    );
    Ok(Cell::instant(Instant::new(
        &r.iso,
        &r.text,
        &r.company,
        &r.utc,
        r.mode.as_str(),
    )))
}
fn actor(user: i64, impersonator: Option<i64>) -> String {
    let mut text = user.to_string();
    if let Some(id) = impersonator {
        text.push_str(&format!(" (impersonated by {id})"));
    }
    text
}
#[derive(Template)]
#[template(path = "operation.html")]
struct Operation<'a> {
    form_dom: &'a str,
    event_name: &'a str,
    button_dom: &'a str,
    verb: &'a str,
    fields: Vec<Field>,
    help: &'a str,
}
fn withheld(id: &str, title: &str, reason: &str) -> Result<Html> {
    let mut empty = EmptyState::missing(title, reason);
    empty.id = id.into();
    render(empty)
}
impl Pages {
    async fn allowed(&self, scope: &Scope, capability: &str) -> Result<bool> {
        let c = self.db.get().await?;
        Ok(bilibi_authz::decide(&**c, scope, capability).await?.allowed)
    }
    async fn display(&self, scope: &Scope) -> Result<DisplayContext> {
        let c = self.db.get().await?;
        let locale = self.locales.resolve(&**c, scope, None).await?.locale;
        self.display.display(&**c, scope, None, locale).await
    }
    fn state(
        &self,
        operation: &str,
        path: &HashMap<String, String>,
        query: &HashMap<String, String>,
        framed: bool,
    ) -> Result<Box<dyn LiveState<Scope, Error>>> {
        if operation == "asset" {
            Ok(Box::new(detail::Detail::new(self, path, framed)?))
        } else {
            Ok(Box::new(index::Index::new(self, operation, query, framed)?))
        }
    }
}
impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "register" => Some("Fixed assets"),
            "categories" => Some("Asset categories"),
            "runs" => Some("Depreciation runs"),
            "asset" => Some("Asset"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = request.scope.ok_or(Error::Unauthorized)?;
            let mut state = self.state(
                request.operation,
                request.path,
                request.query,
                request.framed,
            )?;
            let html = state
                .update(LiveRequest {
                    scope,
                    credential: "",
                    input: LiveInput::Refresh,
                })
                .await?
                .html;
            Ok(PageBody {
                html,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            Ok(Some(self.state(
                request.operation,
                request.path,
                request.query,
                request.framed,
            )?))
        })
    }
}

fn flash_map(flash: &Option<(StatusKind, String)>) -> bilibi_ui::FlashMap<'_> {
    let mut messages = bilibi_ui::NO_FLASH;
    if let Some((kind, text)) = flash {
        match kind {
            StatusKind::Error => messages.error = text,
            StatusKind::Success => messages.success = text,
            StatusKind::Warning => messages.warning = text,
            StatusKind::Info => messages.info = text,
        }
    }
    messages
}
