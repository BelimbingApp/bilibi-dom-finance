//! Bilimbi Finance.Payables.Web at 4abfaf829321f8540581d833752a086b21b45ba9.
//! Pages only compose shared components and call the authorized facade.
mod bill;
mod bills;
mod format;

use crate::{Payables, MODULE, VIEW};
use askama::Template;
use bilibi_database::{Db, Error, Result};
use bilibi_datetime::{DisplayContext, DisplayPolicy};
use bilibi_locale::Locales;
use bilibi_module_registry::{
    contributions::{Access, Method, Route, ROUTES},
    Contribution, Provenance,
};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{MountFuture, Page, PageBody, PageFuture, PageRequest},
    Html,
};
use std::sync::Arc;

pub(crate) const PAGES: &str = "finance/payables.pages";
const INDEX: &str = "/finance/payables/bills";

#[derive(Clone)]
pub(crate) struct Pages {
    pub db: Db,
    pub payables: Arc<Payables>,
    pub display: Arc<DisplayPolicy>,
    pub locales: Arc<Locales>,
}

bilibi_menu::contribute!(
    "finance/payables",
    bilibi_menu::MenuItem::page("finance.payables.bills", "Bills", INDEX)
        .icon("clipboard-document-list")
        .under("finance")
        .requires(bilibi_menu::Requirement::Capability(VIEW))
        .order(5)
);

macro_rules! route {
    ($key:literal, $path:expr, $operation:literal) => {
        inventory::submit! { Contribution::new(ROUTES, $key,
        Provenance { module: MODULE, source: file!(), line: line!() }, &Route {
            method: Method::Get, path: $path, service: PAGES,
            operation: $operation, access: Access::Capability(VIEW), framed: true,
        }) }
    };
}
route!("finance/payables.bills", INDEX, "bills");
route!(
    "finance/payables.bill",
    "/finance/payables/bills/{id}",
    "bill"
);

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
            "bills" => Some("Bills"),
            "bill" => Some("Bill"),
            _ => None,
        }
    }

    fn render<'a>(&'a self, r: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let scope = r.scope.ok_or(Error::Unauthorized)?;
            let (html, title) = match r.operation {
                "bills" => {
                    let mut state = bills::Bills::new(self.clone(), r.query, r.framed)?;
                    state.reload(scope).await?;
                    (state.body()?, None)
                }
                "bill" => {
                    let mut state = bill::Bill::new(self.clone(), r.path)?;
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
                "bills" => Box::new(bills::Bills::new(self.clone(), r.query, r.framed)?),
                "bill" => Box::new(bill::Bill::new(self.clone(), r.path)?),
                _ => return Err(Error::NotFound),
            };
            Ok(Some(state))
        })
    }
}

fn company(scope: &Scope) -> Option<i64> {
    scope.actor().organisation().map(|id| id.get())
}

fn render(fragment: impl Template) -> Result<Html> {
    Html::render(&fragment).map_err(|_| Error::Internal)
}
