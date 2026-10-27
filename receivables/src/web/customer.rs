//! One customer: its details and address, edited in place, with the invoices
//! and receipts that belong to it. Reads `customer.get`, `document.list` and
//! `payment.list`; each edit calls `customer.update`.
use super::{
    format::{self as f},
    render, Flash, Pages, Written, CUSTOMERS, INVOICES, RECEIPTS,
};
use crate::{Customer as Record, Document, Payment, MANAGE_CUSTOMERS};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, BackLink, Card, Cell, CommitStates, CommitStatus, EmptyState, Facts, Field, FlexRow,
    Header, Html, InlineChoice, InlineEdit, InputKind, ListColumns, PageVariant, SectionHeading,
    StatusKind,
};
use serde_json::json;
use std::collections::HashMap;

/// `(key, label)` of the facts a customer edits in place, by card.
const DETAILS: &[(&str, &str)] = &[
    ("name", "Name"),
    ("email", "Email"),
    ("phone", "Phone"),
    ("payment_terms_days", "Payment terms (days)"),
    ("tax_identifier", "Tax ID"),
    ("registration_identifier", "Registration number"),
];
const ADDRESS: &[(&str, &str)] = &[
    ("line1", "Address line 1"),
    ("line2", "Address line 2"),
    ("line3", "Address line 3"),
    ("locality", "City"),
    ("postcode", "Postcode"),
    ("admin1_code", "State or region"),
    ("country_iso", "Country code"),
];

pub(super) struct Customer {
    pages: Pages,
    id: i64,
    record: Option<Record>,
    documents: Vec<Document>,
    payments: Vec<Payment>,
    can_edit: bool,
    statuses: CommitStates,
    flash: Flash,
    figures: f::Figures,
    invoices: ListColumns,
    receipts: ListColumns,
}

impl Customer {
    pub(super) fn new(pages: Pages, path: &HashMap<String, String>) -> Result<Self> {
        Ok(Self {
            pages,
            id: params::positive_id(path.get("id").map(String::as_str)).unwrap_or(0),
            record: None,
            documents: vec![],
            payments: vec![],
            can_edit: false,
            statuses: CommitStates::default(),
            flash: Flash::default(),
            figures: f::Figures::default(),
            invoices: f::columns(
                "receivables-customer-invoices-table",
                &[
                    ("number", "Number"),
                    ("kind", "Type"),
                    ("issue_date", "Issued"),
                    ("due_date", "Due"),
                    ("total", "Total"),
                    ("outstanding", "Outstanding"),
                    ("status", "Status"),
                ],
                &["total", "outstanding"],
            )?,
            receipts: f::columns(
                "receivables-customer-receipts-table",
                &[
                    ("number", "Receipt"),
                    ("payment_date", "Date"),
                    ("amount", "Amount"),
                    ("status", "Status"),
                ],
                &["amount"],
            )?,
        })
    }

    pub(super) fn title(&self) -> Option<String> {
        self.record.as_ref().map(|c| c.name.clone())
    }

    pub(super) async fn first(&mut self, scope: &Scope) -> Result<String> {
        self.load(scope).await?;
        self.body()
    }

    async fn load(&mut self, scope: &Scope) -> Result<()> {
        self.record = match self
            .pages
            .read(
                scope,
                "finance.receivables.customer.get",
                json!({"customer_id": self.id}),
            )
            .await
        {
            Ok(value) => serde_json::from_value(value).ok(),
            Err(Error::NotFound) => None,
            Err(error) => return Err(error),
        };
        self.can_edit = self.pages.allowed(scope, MANAGE_CUSTOMERS).await?;
        self.figures = self.pages.figures(scope).await;
        if self.record.is_some() {
            self.documents = self
                .pages
                .read_all(
                    scope,
                    "finance.receivables.document.list",
                    json!({"customer_id": self.id}),
                    ("after", "next_after"),
                )
                .await?
                .0;
            self.payments = self
                .pages
                .read_all(
                    scope,
                    "finance.receivables.payment.list",
                    json!({"customer_id": self.id}),
                    ("after", "next_after"),
                )
                .await?
                .0;
            self.documents.reverse();
            self.payments.reverse();
            self.invoices.load(&self.documents, |d| d.id.to_string());
            self.receipts.load(&self.payments, |p| p.id.to_string());
        }
        Ok(())
    }

    fn value(c: &Record, key: &str) -> String {
        let text = |t: &Option<String>| t.clone().unwrap_or_default();
        match key {
            "name" => c.name.clone(),
            "email" => text(&c.email),
            "phone" => text(&c.phone),
            "payment_terms_days" => c.payment_terms_days.to_string(),
            "tax_identifier" => text(&c.tax_identifier),
            "registration_identifier" => text(&c.registration_identifier),
            "line1" => text(&c.line1),
            "line2" => text(&c.line2),
            "line3" => text(&c.line3),
            "locality" => text(&c.locality),
            "postcode" => text(&c.postcode),
            "admin1_code" => text(&c.admin1_code),
            "country_iso" => text(&c.country_iso),
            _ => String::new(),
        }
    }

    fn facts(&self, c: &Record, id: &str, rows: &[(&'static str, &'static str)]) -> Result<Html> {
        let mut items = Vec::new();
        for (key, label) in rows {
            let mut field = f::inline(
                key,
                label,
                &Self::value(c, key),
                c.id,
                "save_customer",
                self.can_edit,
                &self.statuses,
            );
            if *key == "name" {
                field.allow_empty = false;
            }
            items.push(f::fact_html(label, render(InlineEdit { field })?));
        }
        render(Facts { id, items })
    }

    fn body(&self) -> Result<String> {
        let back = render(BackLink {
            href: CUSTOMERS,
            id: "receivables-customer-back",
            title: "Back to customers",
            ..BackLink::default()
        })?;
        let Some(c) = &self.record else {
            let mut missing = EmptyState::missing(
                "Customer not found",
                "There is no such customer in this company, or you cannot read it.",
            );
            missing.id = "receivables-customer-missing".into();
            missing.action = back;
            return f::frame(
                "receivables-customer",
                PageVariant::Detail,
                &self.flash,
                render(missing)?,
                None,
            );
        };
        let owed = self
            .documents
            .iter()
            .filter(|d| d.kind == "invoice" && d.status == "issued")
            .filter_map(|d| d.outstanding)
            .fold(rust_decimal::Decimal::ZERO, |sum, m| sum + m.amount());
        let currency = self
            .documents
            .first()
            .map_or("", |d| d.currency_code.as_str());
        let mut status_editor = Field::named("active")
            .id("receivables-customer-active-editor")
            .label_class("sr-only")
            .label("Status")
            .kind(InputKind::Select)
            .value(if c.active { "true" } else { "false" })
            .options(bilibi_ui::choices(&[
                ("Active", "true"),
                ("Inactive", "false"),
            ]));
        status_editor = status_editor.aria_label("Status");
        let mut status_field = f::inline(
            "active",
            "Status",
            if c.active { "true" } else { "false" },
            c.id,
            "save_customer",
            self.can_edit,
            &self.statuses,
        );
        status_field.allow_empty = false;
        let mut details = self.facts(c, "receivables-customer-details", DETAILS)?;
        let mut items = vec![
            f::fact("Code", &c.code),
            f::fact_html(
                "Status",
                render(InlineChoice {
                    field: status_field,
                    display: f::active_badge(c.active)?,
                    editor: render(status_editor)?,
                })?,
            ),
            f::fact(
                "Owed",
                &if owed.is_zero() {
                    "Nothing outstanding".into()
                } else {
                    format!("{} {currency}", self.figures.decimal(owed))
                },
            ),
        ];
        let mut summary = render(Facts {
            id: "receivables-customer-summary",
            items: std::mem::take(&mut items),
        })?;
        summary.push(&details);
        details = summary;
        let identity = render(Card {
            id: "receivables-customer-identity",
            role: "region",
            body: {
                let mut b = render(SectionHeading {
                    title: "Details",
                    ..SectionHeading::default()
                })?;
                b.push(&details);
                b
            },
            ..Card::default()
        })?;
        let address = render(Card {
            id: "receivables-customer-address",
            role: "region",
            class: "mt-4",
            body: {
                let mut b = render(SectionHeading {
                    title: "Address",
                    description: "Printed on invoices you issue from now on. Invoices already issued keep the address they were issued with.",
                    ..SectionHeading::default()
                })?;
                b.push(&self.facts(c, "receivables-customer-address-facts", ADDRESS)?);
                b
            },
            ..Card::default()
        })?;
        let mut invoice_rows = Vec::new();
        for d in &self.documents {
            let mut row = FlexRow::new(d.id.to_string());
            row.dom_id = Some(format!("receivables-customer-invoice-{}", d.id));
            invoice_rows.push(
                row.slot(
                    "number",
                    f::link(
                        &format!("{INVOICES}/{}", d.id),
                        &super::documents::reference(d),
                    )?,
                )
                .cell("kind", Cell::text(f::kind_label(&d.kind)))
                .cell("issue_date", Cell::text(f::optional(&d.issue_date)))
                .cell("due_date", Cell::text(f::optional(&d.due_date)))
                .slot("total", f::tabular(&self.figures.money(d.total))?)
                .slot(
                    "outstanding",
                    f::tabular(&self.figures.optional(d.outstanding))?,
                )
                .slot("status", f::status_badge(&d.status)?),
            );
        }
        let mut receipt_rows = Vec::new();
        for p in &self.payments {
            let mut row = FlexRow::new(p.id.to_string());
            row.dom_id = Some(format!("receivables-customer-receipt-{}", p.id));
            receipt_rows.push(
                row.slot(
                    "number",
                    f::link(&format!("{RECEIPTS}/{}", p.id), &p.payment_reference)?,
                )
                .cell("payment_date", Cell::text(&p.payment_date))
                .slot("amount", f::tabular(&self.figures.money(p.amount))?)
                .slot("status", super::payments::status_badge(p)?),
            );
        }
        let invoices = render(Card {
            id: "receivables-customer-invoices",
            role: "region",
            class: "mt-4",
            inner_class: "p-0",
            body: {
                let mut b = render(SectionHeading {
                    title: "Invoices and credit notes",
                    class: "p-5 pb-0",
                    ..SectionHeading::default()
                })?;
                b.push(&f::table(
                    &self.invoices,
                    "invoices_grid",
                    "Invoices and credit notes",
                    invoice_rows,
                    "Nothing has been invoiced to this customer yet.",
                )?);
                b
            },
            ..Card::default()
        })?;
        let receipts = render(Card {
            id: "receivables-customer-receipts",
            role: "region",
            class: "mt-4",
            inner_class: "p-0",
            body: {
                let mut b = render(SectionHeading {
                    title: "Receipts",
                    class: "p-5 pb-0",
                    ..SectionHeading::default()
                })?;
                b.push(&f::table(
                    &self.receipts,
                    "receipts_grid",
                    "Receipts",
                    receipt_rows,
                    "No receipts have been recorded for this customer.",
                )?);
                b
            },
            ..Card::default()
        })?;
        let mut body = f::announce(c.id)?;
        let pin_name = format!("{} {}", c.code, c.name);
        body.push(&render(Header {
            title: Html::text(&c.name),
            title_actions: super::record_pin(
                "receivables-customer-pin",
                "customer",
                &pin_name,
                &format!("{}/{}", super::CUSTOMERS, c.id),
            )?,
            subtitle: &format!("Customer {}", c.code),
            actions: back,
        })?);
        for part in [identity, address, invoices, receipts] {
            body.push(&part);
        }
        f::frame(
            "receivables-customer",
            PageVariant::Detail,
            &self.flash,
            body,
            None,
        )
    }

    async fn save_field(
        &mut self,
        scope: &Scope,
        credential: &str,
        v: &serde_json::Value,
    ) -> Result<()> {
        let key = v["field"].as_str().ok_or(Error::Invalid)?;
        let label = DETAILS
            .iter()
            .chain(ADDRESS)
            .find(|(k, _)| *k == key)
            .map(|(_, l)| *l)
            .or((key == "active").then_some("Status"))
            .ok_or(Error::Invalid)?;
        if v["id"].as_str() != Some(&self.id.to_string()) {
            return Err(Error::Invalid);
        }
        let submitted = v["value"].as_str().ok_or(Error::Invalid)?.trim().to_owned();
        let mut input = json!({"customer_id": self.id});
        match key {
            "active" => input["active"] = json!(submitted == "true"),
            "payment_terms_days" => match submitted.parse::<i64>() {
                Ok(days) => input[key] = json!(days),
                Err(_) => {
                    self.statuses.put(
                        key,
                        CommitStatus::refusal(
                            label,
                            &submitted,
                            &["must be a whole number of days"],
                        ),
                    );
                    return Ok(());
                }
            },
            "country_iso" => input[key] = json!(submitted.to_uppercase()),
            _ => input[key] = json!(submitted),
        }
        if !self.can_edit {
            self.statuses.clear_saved();
            self.statuses.put(
                key,
                CommitStatus::Error("You do not have permission to change customers.".into()),
            );
            return Ok(());
        }
        let written: Written = self
            .pages
            .write(
                scope,
                credential,
                "save_customer",
                "finance.receivables.customer.update",
                input,
            )
            .await?;
        self.statuses
            .put(key, f::commit_status(&written, key, label, &submitted));
        Ok(())
    }
}

impl LiveState<Scope, Error> for Customer {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh | LiveInput::Params(_) => {}
                LiveInput::Event(v) => match v["type"].as_str().ok_or(Error::Invalid)? {
                    "clear-flash" => self.flash.dismiss(v),
                    "invoices_grid" => {
                        self.invoices.handle(&params::string_values(v));
                    }
                    "receipts_grid" => {
                        self.receipts.handle(&params::string_values(v));
                    }
                    "save_customer" => self.save_field(r.scope, r.credential, v).await?,
                    _ => return Err(Error::Invalid),
                },
            }
            self.load(r.scope).await?;
            if self.record.is_none() {
                self.flash
                    .set(StatusKind::Error, "That customer is no longer available.");
            }
            Ok(LiveBody {
                html: self.body()?,
                ..LiveBody::default()
            })
        })
    }
}
