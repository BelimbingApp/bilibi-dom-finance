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
    pub contracts: Arc<bilibi_agent_api::page::PageContracts>,
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
// Menu group (a bookkeeper's section of the Finance menu); its pages sit under it.
bilibi_menu::contribute!(
    "finance/fixed_assets",
    bilibi_menu::MenuItem::container("finance.group.fixed-assets", "Fixed assets")
        .under("finance")
        .icon("carbon-financial-assets")
        .order(40)
);

macro_rules! menu {
    ($kind:literal, $label:literal, $order:literal) => {
        bilibi_menu::contribute!(
            "finance/fixed_assets",
            bilibi_menu::MenuItem::page(
                concat!("finance.fixed-assets.", $kind),
                $label,
                concat!("/finance/fixed-assets/", $kind)
            )
            .icon("carbon-financial-assets")
            .under("finance.group.fixed-assets")
            .requires(bilibi_menu::Requirement::Capability(crate::VIEW))
            .order($order)
        );
    };
}
menu!("register", "Fixed assets", 1);
menu!("runs", "Depreciation runs", 2);
menu!("categories", "Asset categories", 3);

fn render(t: impl Template) -> Result<Html> {
    Html::render(&t).map_err(|_| Error::Internal)
}

/// Sidebar pin for a menu page. An asset the page could not load is not pinned.
pub(super) fn menu_pin(id: &str, nav: &str, page: &str) -> Result<Html> {
    let label = format!("Pin {page} to sidebar");
    render(bilibi_ui::IconButton {
        context: bilibi_ui::IconContext::Inline,
        id,
        nav_pin: nav,
        ..bilibi_ui::IconButton::new("bilimbi-pin", &label)
    })
}

pub(super) fn record_pin(id: &str, noun: &str, name: &str, url: &str) -> Result<Html> {
    let label = format!("Pin this {noun} to sidebar");
    let pin_label = name.replace('/', " ");
    render(RecordPin {
        id,
        label: &label,
        pin_label: &pin_label,
        url,
    })
}

#[derive(Template)]
#[template(
    source = r#"{% import "components.html" as ui %}{% call ui::icon_button(icon="bilimbi-pin", label=label, context=bilibi_ui::IconContext::Inline, id=id, nav_pin="record", record_label=pin_label, record_url=url) %}{% endcall %}"#,
    ext = "html"
)]
struct RecordPin<'a> {
    id: &'a str,
    label: &'a str,
    pin_label: &'a str,
    url: &'a str,
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
        .parse::<jiff::civil::DateTime>()
        .map_err(|_| Error::Invalid)?;
    // The schema stores naive UTC, as DisplayContext::render_naive_utc defines.
    let r = display
        .render_naive_utc(
            at,
            bilibi_datetime::Format::DateTime,
            bilibi_datetime::Precision::Minute,
        )
        .ok_or(Error::Invalid)?;
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

// A delayed success timeout must not dismiss a newer refusal.
fn clear_flash(flash: &mut Option<(StatusKind, String)>, value: &serde_json::Value) {
    if flash
        .as_ref()
        .is_some_and(|(kind, _)| value["key"].as_str() == Some(kind.name()))
    {
        *flash = None;
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

// ADR 01 gap, listed so it stays visible: these pages still write through
// the facade rather than an operation contract (`base/agent_api` page).
inventory::submit! { bilibi_module_registry::Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, PAGES, bilibi_module_registry::Provenance { module: MODULE, source: file!(), line: line!() }, &bilibi_module_registry::contributions::PageWrites {
    service: PAGES,
    contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Registering, disposing, reversing and voiding assets write through the Fixed Assets facade; no contract yet." },
}) }

impl Pages {
    async fn account_context(
        &self,
        scope: &Scope,
    ) -> Result<(
        Option<bilibi_finance_ledger::MinorUnits>,
        Vec<(i64, String, String, bool)>,
    )> {
        let Some(company) = scope.actor().organisation().map(|c| c.get()) else {
            return Ok((None, Vec::new()));
        };
        let mut input = serde_json::json!({"company_id":company});
        let book = match self
            .contracts
            .read(scope, "finance.ledger.book.get", &input)
            .await
        {
            Ok(book) => book,
            // A document viewer need not have permission to read the chart.
            Err(
                bilibi_agent_api::Error::Forbidden
                | bilibi_agent_api::Error::Backend(Error::Forbidden | Error::Unauthorized),
            ) => return Ok((None, Vec::new())),
            Err(_) => return Err(Error::Internal),
        };
        let units = book["result"]["book"]["minor_units"]
            .as_i64()
            .and_then(bilibi_finance_ledger::MinorUnits::new);
        let mut accounts = Vec::new();
        loop {
            let page = self
                .contracts
                .read(scope, "finance.ledger.account.list", &input)
                .await
                .map_err(|_| Error::Internal)?;
            for a in page["result"]["records"].as_array().into_iter().flatten() {
                if let (Some(id), Some(code), Some(name), Some(kind), Some(active)) = (
                    a["id"].as_i64(),
                    a["code"].as_str(),
                    a["name"].as_str(),
                    a["account_type"].as_str(),
                    // A group takes no postings, so it is never a choice.
                    a["active"]
                        .as_bool()
                        .map(|active| active && a["is_group"].as_bool() != Some(true)),
                ) {
                    accounts.push((id, format!("{code} {name}"), kind.to_owned(), active));
                }
            }
            if let Some(code) = page["result"]["next_after_code"].as_str() {
                input["after_code"] = serde_json::json!(code);
            } else {
                break;
            }
        }
        Ok((units, accounts))
    }
}
