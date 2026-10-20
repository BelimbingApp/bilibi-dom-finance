//! Bilimbi Finance.Payables.Web at 4abfaf829321f8540581d833752a086b21b45ba9.
//! Pages only compose shared components and call the authorized facade.
mod aged;
mod bill;
mod bill_form;
mod bills;
mod format;
mod payment;
mod payment_form;
mod payments;
mod setup;

use crate::{Payables, CREATE_DOCUMENTS, MODULE, SUBMIT_PAYMENTS, VIEW};
use askama::Template;
use bilibi_agent_api::{
    page::{Call, PageContracts},
    Error as ContractError,
};
use bilibi_database::{Db, Error, Result};
use bilibi_datetime::{Clock, DisplayContext, DisplayPolicy};
use bilibi_finance_ledger::Ledger;
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
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) const PAGES: &str = "finance/payables.pages";
const INDEX: &str = "/finance/payables/bills";
const NEW_BILL: &str = "/finance/payables/bills/new";
const NEW_NOTE: &str = "/finance/payables/debit-notes/new";
const PAYMENTS: &str = "/finance/payables/payments";
const NEW_PAYMENT: &str = "/finance/payables/payments/new";
const AGED: &str = "/finance/payables/aged";
const SETUP: &str = "/finance/payables/setup";

#[derive(Clone)]
pub(crate) struct Pages {
    pub db: Db,
    pub payables: Arc<Payables>,
    pub ledger: Arc<Ledger>,
    pub contracts: Arc<PageContracts>,
    pub display: Arc<DisplayPolicy>,
    pub locales: Arc<Locales>,
}

// Menu group (a bookkeeper's section of the Finance menu); its pages sit under it.
bilibi_menu::contribute!(
    "finance/payables",
    bilibi_menu::MenuItem::container("finance.group.payables", "Payables")
        .under("finance")
        .order(20)
);

macro_rules! menu {
    ($key:literal, $title:literal, $path:expr, $icon:literal, $order:literal, $capability:expr) => {
        bilibi_menu::contribute!(
            "finance/payables",
            bilibi_menu::MenuItem::page($key, $title, $path)
                .icon($icon)
                .under("finance.group.payables")
                .requires(bilibi_menu::Requirement::Capability($capability))
                .order($order)
        );
    };
}
menu!(
    "finance.payables.bills",
    "Bills",
    INDEX,
    "clipboard-document-list",
    1,
    VIEW
);
menu!(
    "finance.payables.payments",
    "Payments",
    PAYMENTS,
    "banknotes",
    2,
    VIEW
);
menu!(
    "finance.payables.aged",
    "Aged payables",
    AGED,
    "calendar-days",
    3,
    VIEW
);
menu!(
    "finance.payables.setup",
    "Payables setup",
    SETUP,
    "cog-6-tooth",
    4,
    VIEW
);

macro_rules! route {
    ($key:literal, $path:expr, $operation:literal, $capability:expr) => {
        inventory::submit! { Contribution::new(ROUTES, $key,
        Provenance { module: MODULE, source: file!(), line: line!() }, &Route {
            method: Method::Get, path: $path, service: PAGES,
            operation: $operation, access: Access::Capability($capability), framed: true,
        }) }
    };
}
route!("finance/payables.bills", INDEX, "bills", VIEW);
route!(
    "finance/payables.bill",
    "/finance/payables/bills/{id}",
    "bill",
    VIEW
);
route!(
    "finance/payables.bill-new",
    NEW_BILL,
    "bill_new",
    CREATE_DOCUMENTS
);
route!(
    "finance/payables.note-new",
    NEW_NOTE,
    "note_new",
    CREATE_DOCUMENTS
);
route!("finance/payables.payments", PAYMENTS, "payments", VIEW);
route!(
    "finance/payables.payment-new",
    NEW_PAYMENT,
    "payment_new",
    SUBMIT_PAYMENTS
);
route!(
    "finance/payables.payment",
    "/finance/payables/payments/{id}",
    "payment",
    VIEW
);
route!("finance/payables.aged", AGED, "aged", VIEW);
route!("finance/payables.setup", SETUP, "setup", VIEW);

/// What a page write came back as. The page words each case for its own
/// action; `Fields` carries the owner's refusals keyed by input path
/// (`lines.0.amount`), and `base` for one that names no field.
pub(super) enum Written {
    Done(Value),
    Fields(BTreeMap<String, Vec<String>>),
    Forbidden,
    Changed,
}

/// The ID of the record a write contract answered with
/// (`{"result": {"record": {...}}}`).
fn record_id(answer: &Value) -> Option<i64> {
    answer["result"]["record"]["id"].as_i64()
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

    /// The company's calendar day, for a date a form starts from.
    async fn today(&self, scope: &Scope) -> Result<jiff::civil::Date> {
        let display = self.display(scope).await?;
        Ok(Clock::resolve(&display, None, jiff::Timestamp::now()).today)
    }

    /// One of these pages' write events, as the contract it declared.
    fn call<'a>(
        &self,
        scope: &'a Scope,
        credential: &'a str,
        event: &'a str,
        contract: &'static str,
    ) -> Call<'a> {
        Call {
            scope,
            credential,
            service: PAGES,
            event,
            contract,
        }
    }

    /// Reviews and commits a form the person has just filled in, as the
    /// contract the agent doors call. Only a lost sign-in (or a step-up the
    /// host answers itself) is an `Err`; every other refusal is a `Written`
    /// the page keeps on the screen.
    async fn write(
        &self,
        scope: &Scope,
        credential: &str,
        event: &str,
        contract: &'static str,
        input: Value,
    ) -> Result<Written> {
        match self
            .contracts
            .submit(&self.call(scope, credential, event, contract), input)
            .await
        {
            Ok(value) => Ok(Written::Done(value)),
            Err(ContractError::InvalidInput(fields)) => Ok(Written::Fields(fields)),
            Err(ContractError::Forbidden | ContractError::Backend(Error::Forbidden)) => {
                Ok(Written::Forbidden)
            }
            Err(ContractError::Backend(
                error @ (Error::Unauthorized | Error::StepUpRequired { .. }),
            )) => Err(error),
            Err(ContractError::NotFound | ContractError::Backend(Error::NotFound)) => {
                Ok(Written::Fields(BTreeMap::from([(
                    "base".into(),
                    vec!["It no longer exists or you cannot reach it.".into()],
                )])))
            }
            Err(ContractError::Backend(Error::Internal)) => Err(Error::Internal),
            Err(_) => Ok(Written::Changed),
        }
    }
}

impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "bills" => Some("Bills"),
            "bill" => Some("Bill"),
            "bill_new" => Some("Record a bill"),
            "note_new" => Some("Record a debit note"),
            "payments" => Some("Payments"),
            "payment_new" => Some("Request a payment"),
            "payment" => Some("Payment request"),
            "aged" => Some("Aged payables"),
            "setup" => Some("Payables setup"),
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
                "bill_new" | "note_new" => {
                    let mut state =
                        bill_form::BillForm::new(self.clone(), r.operation == "note_new", r.query)?;
                    state.reload(scope).await?;
                    (state.body()?, None)
                }
                "payments" => {
                    let mut state = payments::Payments::new(self.clone(), r.query, r.framed)?;
                    state.reload(scope).await?;
                    (state.body()?, None)
                }
                "payment_new" => {
                    let mut state = payment_form::PaymentForm::new(self.clone(), r.query)?;
                    state.reload(scope).await?;
                    (state.body()?, None)
                }
                "payment" => {
                    let mut state = payment::Request::new(self.clone(), r.path)?;
                    state.reload(scope).await?;
                    (state.body(scope).await?, state.title())
                }
                "aged" => {
                    let mut state = aged::Aged::new(self.clone(), r.query)?;
                    state.reload(scope).await?;
                    (state.body(scope).await?, None)
                }
                "setup" => {
                    let mut state = setup::Setup::new(self.clone())?;
                    state.reload(scope).await?;
                    (state.body()?, None)
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
                "bill_new" | "note_new" => Box::new(bill_form::BillForm::new(
                    self.clone(),
                    r.operation == "note_new",
                    r.query,
                )?),
                "payments" => Box::new(payments::Payments::new(self.clone(), r.query, r.framed)?),
                "payment_new" => Box::new(payment_form::PaymentForm::new(self.clone(), r.query)?),
                "payment" => Box::new(payment::Request::new(self.clone(), r.path)?),
                "aged" => Box::new(aged::Aged::new(self.clone(), r.query)?),
                "setup" => Box::new(setup::Setup::new(self.clone())?),
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

// Every write these pages make is a Payables contract (ADR 01), the same
// operation the agent doors call. Approving match exceptions has no contract
// yet (`guide.md`: no match-override operation), so it stays a listed
// allowance until one exists.
inventory::submit! { bilibi_module_registry::Contribution::new(bilibi_module_registry::contributions::PAGE_WRITES, PAGES, bilibi_module_registry::Provenance { module: MODULE, source: file!(), line: line!() }, &bilibi_module_registry::contributions::PageWrites {
    service: PAGES,
    contracts: &[
        ("approve", "finance.payables.document.approve"),
        ("post", "finance.payables.document.post"),
        ("void", "finance.payables.document.void"),
        ("save", "finance.payables.bill.create"),
        ("save", "finance.payables.debit_note.create"),
        ("save", "finance.payables.document.void"),
        ("save_payment", "finance.payables.payment.submit"),
        ("approve_payment", "finance.payables.payment.approve"),
        ("reject_payment", "finance.payables.payment.reject"),
        ("reverse_payment", "finance.payables.payment.reverse"),
        ("save_settings", "finance.payables.settings.put"),
        ("save_tax_code", "finance.payables.tax_code.create"),
        ("toggle_tax_code", "finance.payables.tax_code.update"),
    ],
    pending: bilibi_module_registry::contributions::Pending::Events {
        events: &["approve_exception"],
        reason: "Approving match exceptions writes through the Payables facade: there is no match-override contract yet.",
    },
}) }
