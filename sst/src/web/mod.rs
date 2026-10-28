//! Bilimbi Finance.Sst.Web at 4abfaf829321f8540581d833752a086b21b45ba9
//! (`web_routes.exs` and the menu of `Contributions`): the Returns, Return and
//! Setup pages, the CSV download and the print page. Pages compose Base UI's
//! components only and call the authorized facade, which asks again.
mod detail;
mod export;
mod format;
mod returns;
mod setup;

pub(crate) use export::Downloads;

use crate::{Sst, MODULE, VIEW};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_datetime::{DisplayContext, DisplayPolicy, Format, Precision};
use bilibi_finance_ledger::Ledger;
use bilibi_locale::Locales;
use bilibi_module_registry::{
    contributions::{Access, Download, Method, Route, DOWNLOADS, ROUTES},
    Contribution, Provenance,
};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{MountFuture, Page, PageBody, PageFuture, PageRequest},
    BuiltinColumn, FlashMap, FlexRow, FlexTable, Html, ListColumns, StatusKind, TableEmpty,
    NO_FLASH,
};
use std::sync::Arc;

/// The service ids the registry resolves for the routes below.
pub(crate) const PAGES: &str = "finance/sst.pages";
pub(crate) const FILES: &str = "finance/sst.files";
const INDEX: &str = "/finance/sst";
const SETUP: &str = "/finance/sst/setup";

/// Bilimbi `LiveAuthorization`'s denied message.
const DENIED: &str = "You do not have permission for that action.";

#[derive(Clone)]
pub(crate) struct Pages {
    pub db: Db,
    pub sst: Arc<Sst>,
    pub ledger: Arc<Ledger>,
    pub display: Arc<DisplayPolicy>,
    pub locales: Arc<Locales>,
}

// Bilimbi `Contributions.menu`: id, label, icon, parent, route, capability, order.
// Menu group (a bookkeeper's section of the Finance menu); its pages sit under it.
// Tax uses Tabler `receipt-tax` (MIT). Shopify Polaris `polaris-tax-icon` is
// not vendored: its licence only allows apps that interoperate with Shopify.
bilibi_menu::contribute!(
    "finance/sst",
    bilibi_menu::MenuItem::container("finance.group.tax", "Tax")
        .under("finance")
        .icon("tabler-receipt-tax")
        .order(50)
);

bilibi_menu::contribute!(
    "finance/sst",
    bilibi_menu::MenuItem::page("finance.sst", "SST returns", INDEX)
        .icon("tabler-receipt-tax")
        .under("finance.group.tax")
        .requires(bilibi_menu::Requirement::Capability(VIEW))
        .order(1)
);
bilibi_menu::contribute!(
    "finance/sst",
    bilibi_menu::MenuItem::page("finance.sst.setup", "SST setup", SETUP)
        .icon("tabler-receipt-tax")
        .under("finance.group.tax")
        .requires(bilibi_menu::Requirement::Capability(VIEW))
        .order(2)
);

macro_rules! provenance {
    () => {
        Provenance {
            module: MODULE,
            source: file!(),
            line: line!(),
        }
    };
}
macro_rules! route {
    ($key:literal, $path:expr, $operation:literal) => {
        inventory::submit! { Contribution::new(ROUTES, $key, provenance!(), &Route {
            method: Method::Get, path: $path, service: PAGES,
            operation: $operation, access: Access::Capability(VIEW), framed: true,
        }) }
    };
}
route!("finance/sst.returns", INDEX, "returns");
route!("finance/sst.setup", SETUP, "setup");
route!("finance/sst.return", "/finance/sst/returns/{id}", "return");
inventory::submit! { Contribution::new(DOWNLOADS, "finance/sst.export", provenance!(), &Download {
    path: "/finance/sst/returns/{id}/export.csv", service: FILES,
    operation: "csv", access: Access::Capability(VIEW),
}) }
inventory::submit! { Contribution::new(DOWNLOADS, "finance/sst.print", provenance!(), &Download {
    path: "/finance/sst/returns/{id}/print", service: FILES,
    operation: "print", access: Access::Capability(VIEW),
}) }

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
}

impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "returns" => Some("SST returns"),
            "setup" => Some("SST setup"),
            "return" => Some("SST return"),
            _ => None,
        }
    }

    fn render<'a>(&'a self, r: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = r.scope.ok_or(Error::Unauthorized)?;
            let (html, title) = match r.operation {
                "returns" => {
                    let mut state = returns::Returns::new(self.clone())?.framed(r.framed);
                    state.reload(scope).await?;
                    (state.body()?, None)
                }
                "setup" => {
                    let mut state = setup::Setup::new(self.clone())?;
                    state.reload(scope).await?;
                    state.reset_registration_form();
                    (state.body(scope).await?, None)
                }
                "return" => {
                    let mut state = detail::Return::new(self.clone(), r.path)?;
                    state.reload(scope).await?;
                    (state.body(scope).await?, state.title())
                }
                _ => return Err(Error::NotFound),
            };
            Ok(PageBody {
                html,
                live: true,
                title,
            })
        })
    }

    fn mount<'a>(&'a self, r: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            let state: Box<dyn bilibi_ui::page::LiveState<Scope, Error>> = match r.operation {
                "returns" => Box::new(returns::Returns::new(self.clone())?.framed(r.framed)),
                "setup" => Box::new(setup::Setup::new(self.clone())?),
                "return" => {
                    let mut state = detail::Return::new(self.clone(), r.path)?;
                    state.subscribe(r.scope.ok_or(Error::Unauthorized)?).await;
                    Box::new(state)
                }
                _ => return Err(Error::NotFound),
            };
            Ok(Some(state))
        })
    }
}

/// The session's current company (Bilibi ADR 09); it grants nothing.
fn company(scope: &Scope) -> Option<i64> {
    scope.current_company().map(|id| id.get())
}

fn render(fragment: impl Template) -> Result<Html> {
    Html::render(&fragment).map_err(|_| Error::Internal)
}

/// Sidebar pin for a menu page. A return the page could not load, and the
/// print and CSV downloads, are not pinned.
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

/// The one message a page shows, as Bilimbi's `put_flash`.
fn flash(message: &Option<(StatusKind, String)>) -> FlashMap<'_> {
    let mut flash = NO_FLASH;
    if let Some((kind, text)) = message {
        match kind {
            StatusKind::Success => flash.success = text,
            StatusKind::Warning => flash.warning = text,
            StatusKind::Error => flash.error = text,
            StatusKind::Info => flash.info = text,
        }
    }
    flash
}

/// Clears the page's message when the browser dismisses it.
fn clear_flash(message: &mut Option<(StatusKind, String)>, v: &serde_json::Value) {
    if message
        .as_ref()
        .is_some_and(|(kind, _)| v["key"].as_str() == Some(kind.name()))
    {
        *message = None;
    }
}

/// A list's built-in columns (Bilimbi `<.table>` columns), the right-aligned
/// ones named.
fn columns(id: &str, cols: &[(&str, &str)], right: &[&str]) -> Result<ListColumns> {
    ListColumns::mount(
        id,
        cols.iter()
            .map(|(id, label)| {
                let c = BuiltinColumn::new(id, label);
                if right.contains(id) {
                    c.right()
                } else {
                    c
                }
            })
            .collect(),
    )
    .map_err(|_| Error::Internal)
}

/// A FlexTable inside a `p-0` card, raising `event` for its column settings.
fn table(
    columns: &ListColumns,
    event: &str,
    caption: &str,
    rows: Vec<FlexRow>,
    empty: Option<(&str, &str)>,
    actions: bool,
) -> Result<Html> {
    let mut table = FlexTable::new(&columns.id, columns.flex_columns(), rows);
    table.event = event;
    table.caption = caption;
    table.framed = false;
    table.zoom = columns.zoom;
    table.suggestions = columns.flex_suggestions();
    table.add_query = &columns.add_query;
    table.actions = actions;
    table.empty = empty
        .map(|(title, reason)| TableEmpty::State(bilibi_ui::EmptyState::missing(title, reason)));
    render(table)
}

/// Bilimbi `<.datetime precision={:second}>` of an SST stamp
/// (`YYYY-MM-DDTHH:MM:SS.ffffffZ`).
fn time(display: &DisplayContext, id: &str, at: &str) -> Result<Html> {
    let at: jiff::Timestamp = at.parse().map_err(|_| Error::Internal)?;
    let r = display.render(at, Format::DateTime, Precision::Second);
    render(Time {
        element_id: id,
        iso: r.iso,
        text: r.text,
        company: r.company,
        utc: r.utc,
        mode: r.mode.as_str(),
    })
}

#[derive(Template)]
#[template(
    source = r#"<time id="{{ element_id }}" datetime="{{ iso }}" data-format="datetime" data-precision="second" data-mode="{{ mode }}" data-text-company="{{ company }}" data-text-utc="{{ utc }}" data-follow-shell="true" data-hook="DateTime" class="tabular-nums">{{ text }}</time>"#,
    ext = "html"
)]
struct Time<'a> {
    element_id: &'a str,
    iso: String,
    text: String,
    company: String,
    utc: String,
    mode: &'static str,
}

// ADR 01 gap, listed so it stays visible: these pages still write through
// the facade rather than an operation contract (`base/agent_api` page).
inventory::submit! { bilibi_module_registry::Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, PAGES, bilibi_module_registry::Provenance { module: MODULE, source: file!(), line: line!() }, &bilibi_module_registry::contributions::PageWrites {
    service: PAGES,
    contracts: &[],
    pending: bilibi_module_registry::contributions::Pending::Every { reason: "Setup, mappings, return adjustments, finalising and withdrawing returns write through the SST facade; no contract yet." },
}) }
