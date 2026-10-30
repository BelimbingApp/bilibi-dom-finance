//! The invoices and credit notes list, and its New invoice and New credit
//! note dialogs. Reads `document.list`, `customer.list` and `series.list`;
//! saving a draft calls `draft.create`.
use super::{
    format::{self as f, Errors},
    render, Flash, Pages, Written, INVOICES,
};
use crate::{Customer, Document, Series, MANAGE_DOCUMENTS};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, BuiltinColumn, Button, ButtonVariant, Cell, Choice, ColumnsOutcome, CreateButton,
    EmptyState, Field, Filter, FilterToolbar, FlexRow, FlexTable, Form, Header, Html, InputKind,
    ListColumns, ListSpec, ListState, ModalWidth, PageCounts, PageVariant, Pagination, RecordLink,
    SortDir, StatusKind, TableEmpty, ToolbarControl,
};
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

const SIZES: [u64; 3] = [25, 50, 100];
static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        sortable: BTreeMap::from([
            ("number".into(), SortDir::Desc),
            ("issue_date".into(), SortDir::Desc),
            ("due_date".into(), SortDir::Asc),
            ("total".into(), SortDir::Desc),
        ]),
        default_sort: Some("number".into()),
        page_sizes: SIZES.to_vec(),
        filters: [
            (
                "kind".into(),
                Filter::one_of(&["", "invoice", "credit_note"], ""),
            ),
            (
                "status".into(),
                Filter::one_of(&["", "draft", "issued", "unpaid", "discarded"], ""),
            ),
            ("customer".into(), Filter::string("")),
        ]
        .into(),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("documents list spec")
});

/// What an invoice or credit note is called: its issued reference, or a draft
/// by the number the system keeps for it.
pub(super) fn reference(d: &Document) -> String {
    d.document_reference
        .clone()
        .unwrap_or_else(|| format!("Draft #{}", d.id))
}

/// The dialog to start a draft: which kind, and what the person has typed.
struct Draft {
    kind: &'static str,
    values: HashMap<String, String>,
    errors: Errors,
}

pub(super) struct Documents {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    all: Vec<Document>,
    customers: Vec<Customer>,
    series: Vec<Series>,
    figures: f::Figures,
    cut: bool,
    can_add: bool,
    create: Option<Draft>,
    flash: Flash,
    framed: bool,
}

impl Documents {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("number", "Number").sortable("number"),
            BuiltinColumn::new("kind", "Type"),
            BuiltinColumn::new("customer", "Customer"),
            BuiltinColumn::new("issue_date", "Issued").sortable("issue_date"),
            BuiltinColumn::new("due_date", "Due").sortable("due_date"),
            BuiltinColumn::new("total", "Total")
                .sortable("total")
                .right(),
            BuiltinColumn::new("outstanding", "Outstanding").right(),
            BuiltinColumn::new("status", "Status"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("receivables-invoices-table", columns)
                .map_err(|_| Error::Internal)?,
            all: vec![],
            customers: vec![],
            series: vec![],
            figures: f::Figures::default(),
            cut: false,
            can_add: false,
            create: None,
            flash: Flash::default(),
            framed,
        })
    }

    pub(super) async fn first(&mut self, scope: &Scope) -> Result<String> {
        self.load(scope).await?;
        self.body()
    }

    async fn load(&mut self, scope: &Scope) -> Result<()> {
        (self.all, self.cut) = self
            .pages
            .read_all(
                scope,
                "finance.receivables.document.list",
                json!({}),
                ("after", "next_after"),
            )
            .await?;
        self.customers = self
            .pages
            .read_all(
                scope,
                "finance.receivables.customer.list",
                json!({}),
                ("after", "next_after_id"),
            )
            .await?
            .0;
        self.series = self
            .pages
            .read_all(
                scope,
                "finance.receivables.series.list",
                json!({}),
                ("after_code", "next_after_code"),
            )
            .await?
            .0;
        self.figures = self.pages.figures(scope).await;
        self.can_add = self.pages.allowed(scope, MANAGE_DOCUMENTS).await?;
        Ok(())
    }

    fn customer_name(&self, d: &Document) -> String {
        d.party_name.clone().unwrap_or_else(|| {
            self.customers
                .iter()
                .find(|c| c.id == d.customer_id)
                .map_or_else(
                    || format!("Customer #{}", d.customer_id),
                    |c| c.name.clone(),
                )
        })
    }

    fn shown(&mut self) -> (Vec<Document>, u64) {
        let search = self.list.search.trim().to_lowercase();
        let filter = |key: &str| self.list.filters.get(key).cloned().unwrap_or_default();
        let (kind, status, customer) = (filter("kind"), filter("status"), filter("customer"));
        let mut rows: Vec<&Document> = self
            .all
            .iter()
            .filter(|d| kind.is_empty() || d.kind == kind)
            .filter(|d| match status.as_str() {
                "" => true,
                "unpaid" => {
                    d.kind == "invoice"
                        && d.status == "issued"
                        && d.outstanding.is_some_and(|m| m.is_positive())
                }
                other => d.status == other,
            })
            .filter(|d| customer.is_empty() || d.customer_id.to_string() == customer)
            .filter(|d| {
                search.is_empty()
                    || reference(d).to_lowercase().contains(&search)
                    || self.customer_name(d).to_lowercase().contains(&search)
                    || d.description
                        .as_ref()
                        .is_some_and(|t| t.to_lowercase().contains(&search))
            })
            .collect();
        match self.list.sort_by.as_deref() {
            Some("issue_date") => rows.sort_by(|a, b| a.issue_date.cmp(&b.issue_date)),
            Some("due_date") => rows.sort_by(|a, b| a.due_date.cmp(&b.due_date)),
            Some("total") => rows.sort_by_key(|d| d.total.amount()),
            _ => rows.sort_by_key(|d| d.id),
        }
        if self.list.sort_dir == Some(SortDir::Desc) {
            rows.reverse();
        }
        let total = rows.len() as u64;
        let size = self.list.page_size.max(1);
        self.list.page = self.list.page.clamp(1, total.div_ceil(size).max(1));
        let from = ((self.list.page - 1) * size) as usize;
        (
            rows.into_iter()
                .skip(from)
                .take(size as usize)
                .cloned()
                .collect(),
            total,
        )
    }

    fn body(&mut self) -> Result<String> {
        let framed = self.framed;
        let (shown, total) = self.shown();
        self.columns.load(&shown, |d| d.id.to_string());
        let actions = if self.can_add {
            let mut a = render(CreateButton {
                noun: "Invoice",
                id: "receivables-invoices-add",
                event: "show_create_invoice",
                ..CreateButton::default()
            })?;
            a.push(&render(Button {
                id: "receivables-credit-notes-add",
                button_type: "button",
                event: "show_create_credit_note",
                body: Html::text("New credit note"),
                ..Button::default()
            })?);
            a
        } else {
            Html::EMPTY
        };
        let mut customer_choices = vec![Choice::new("All customers", "")];
        customer_choices.extend(
            self.customers
                .iter()
                .map(|c| Choice::new(format!("{} {}", c.code, c.name), c.id.to_string())),
        );
        let mut content = render(FilterToolbar {
            event: "filter",
            ..FilterToolbar::new(
                "receivables-invoices-filters",
                vec![
                    ToolbarControl::search(
                        "receivables-invoices-search",
                        "search",
                        "Search invoices",
                        &self.list.search,
                        "Search by number, customer or description…",
                    ),
                    ToolbarControl::select(
                        "receivables-invoices-kind",
                        "kind",
                        "Filter by type",
                        &self.list.filters["kind"],
                        [
                            ("All types", ""),
                            ("Invoices", "invoice"),
                            ("Credit notes", "credit_note"),
                        ]
                        .map(|(l, v)| Choice::new(l, v))
                        .to_vec(),
                    ),
                    ToolbarControl::select(
                        "receivables-invoices-status",
                        "status",
                        "Filter by status",
                        &self.list.filters["status"],
                        [
                            ("All statuses", ""),
                            ("Draft", "draft"),
                            ("Issued", "issued"),
                            ("Unpaid", "unpaid"),
                            ("Discarded", "discarded"),
                        ]
                        .map(|(l, v)| Choice::new(l, v))
                        .to_vec(),
                    ),
                    ToolbarControl::select(
                        "receivables-invoices-customer",
                        "customer",
                        "Filter by customer",
                        &self.list.filters["customer"],
                        customer_choices,
                    ),
                ],
            )
        })?;
        let mut rows = Vec::new();
        for d in &shown {
            let mut row = FlexRow::new(d.id.to_string());
            row.dom_id = Some(format!("receivables-invoice-{}", d.id));
            rows.push(
                row.slot(
                    "number",
                    render(RecordLink {
                        workspace: framed,
                        kind: crate::MODULE,
                        record_id: &d.id.to_string(),
                        href: &format!("{INVOICES}/{}", d.id),
                        class: "font-medium text-ink-strong hover:underline",
                        body: Html::text(&reference(d)),
                    })?,
                )
                .cell("kind", Cell::text(f::kind_label(&d.kind)))
                .cell("customer", Cell::text(self.customer_name(d)))
                .cell("issue_date", Cell::text(f::optional(&d.issue_date)))
                .cell("due_date", Cell::text(f::optional(&d.due_date)))
                .slot("total", f::tabular(&self.figures.money(d.total))?)
                .slot(
                    "outstanding",
                    f::tabular(&if d.status == "issued" && d.kind == "invoice" {
                        self.figures.optional(d.outstanding)
                    } else {
                        "—".into()
                    })?,
                )
                .slot("status", f::status_badge(&d.status)?),
            );
        }
        let mut table = FlexTable::new(
            "receivables-invoices-table",
            self.columns.flex_columns(),
            rows,
        )
        .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.caption = "Invoices and credit notes";
        table.framed = false;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(render(EmptyState::missing(
            "No invoices",
            if self.all.is_empty() {
                "Start an invoice for a customer. It stays a draft until you issue it."
            } else {
                "No invoice or credit note matches the current filters."
            },
        ))?));
        content.push(&render(table)?);
        content.push(&render(Pagination {
            filters_event: "filter",
            ..Pagination::new(
                "receivables-invoices-pagination",
                PageCounts::new(self.list.page, self.list.page_size, total),
                &SIZES,
            )
        })?);
        if self.cut {
            content.push(&Html::text(
                "Only the first documents are listed. Search to narrow them.",
            ));
        }
        let mut body = render(Header {
            title: Html::text("Invoices"),
            title_actions: bilibi_ui::Html::EMPTY,
            actions,
            ..Header::default()
        })?;
        body.push(&content);
        let modal = match &self.create {
            Some(draft) => Some(self.dialog(draft)?),
            None => None,
        };
        f::frame(
            "receivables-invoices",
            PageVariant::List,
            &self.flash,
            body,
            modal,
        )
    }

    fn dialog(&self, draft: &Draft) -> Result<Html> {
        let credit = draft.kind == "credit_note";
        let mut labels = vec![
            ("series_id", "Series"),
            ("customer_id", "Customer"),
            ("issue_date", "Issue date"),
            ("due_date", "Due date"),
            ("description", "Description"),
        ];
        if credit {
            labels.push(("credited_document_id", "Credited invoice"));
        }
        let labels: Vec<_> = labels
            .into_iter()
            .map(|(k, l)| (k.to_owned(), format!("receivables-draft-{k}"), l.to_owned()))
            .collect();
        let value = |key: &str| draft.values.get(key).cloned().unwrap_or_default();
        let mut body = render(draft.errors.summary("receivables-draft-errors", &labels))?;
        let series: Vec<Choice> = self
            .series
            .iter()
            .filter(|s| s.kind == draft.kind && s.active)
            .map(|s| Choice::new(format!("{} ({})", s.name, s.code), s.id.to_string()))
            .collect();
        let customers: Vec<Choice> = self
            .customers
            .iter()
            .filter(|c| c.active || credit)
            .map(|c| Choice::new(format!("{} {}", c.code, c.name), c.id.to_string()))
            .collect();
        let mut who = Html::EMPTY;
        who.push(&render(
            Field::in_form("draft", "series_id")
                .id("receivables-draft-series_id")
                .label("Series")
                .kind(InputKind::Select)
                .prompt(if series.is_empty() {
                    "No series yet — add one in Settings"
                } else {
                    "Choose a series"
                })
                .options(series)
                .value(value("series_id"))
                .required()
                .hint("The numbering the document takes when it is issued.")
                .errors(draft.errors.under("series_id", "Series")),
        )?);
        who.push(&render(
            Field::in_form("draft", "customer_id")
                .id("receivables-draft-customer_id")
                .label("Customer")
                .kind(InputKind::Select)
                .prompt("Choose a customer")
                .options(customers)
                .value(value("customer_id"))
                .required()
                .errors(draft.errors.under("customer_id", "Customer")),
        )?);
        body.push(&f::two(who)?);
        if credit {
            let invoices: Vec<Choice> = self
                .all
                .iter()
                .filter(|d| d.kind == "invoice" && d.status == "issued")
                .map(|d| {
                    Choice::new(
                        format!("{} · {}", reference(d), self.customer_name(d)),
                        d.id.to_string(),
                    )
                })
                .collect();
            body.push(&render(
                Field::in_form("draft", "credited_document_id")
                    .id("receivables-draft-credited_document_id")
                    .label("Credited invoice")
                    .kind(InputKind::Select)
                    .prompt("Choose the invoice this credits")
                    .options(invoices)
                    .value(value("credited_document_id"))
                    .required()
                    .hint("It must be an issued invoice to the same customer.")
                    .errors(
                        draft
                            .errors
                            .under("credited_document_id", "Credited invoice"),
                    ),
            )?);
        }
        let mut dates = Html::EMPTY;
        for (key, label, hint) in [
            (
                "issue_date",
                "Issue date",
                "Leave empty to set it when you issue.",
            ),
            (
                "due_date",
                "Due date",
                "Leave empty to use the customer's payment terms.",
            ),
        ] {
            dates.push(&render(
                Field::in_form("draft", key)
                    .id(format!("receivables-draft-{key}"))
                    .label(label)
                    .kind(InputKind::Date)
                    .value(value(key))
                    .hint(hint)
                    .errors(draft.errors.under(key, label)),
            )?);
        }
        body.push(&f::two(dates)?);
        body.push(&render(
            Field::in_form("draft", "description")
                .id("receivables-draft-description")
                .label("Description")
                .kind(InputKind::Textarea)
                .attr("maxlength", "500")
                .value(value("description"))
                .errors(draft.errors.under("description", "Description")),
        )?);
        let mut actions = render(Button {
            id: "receivables-draft-cancel",
            button_type: "button",
            event: "cancel_create",
            body: Html::text("Cancel"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: "receivables-draft-save",
            button_type: "submit",
            variant: ButtonVariant::Primary,
            working: "Saving…",
            body: Html::text("Save draft"),
            ..Button::default()
        })?);
        let form = render(Form {
            id: "receivables-draft-form",
            event: "create_draft",
            change: false,
            class: "mt-4",
            body,
            actions: f::actions(actions)?,
        })?;
        f::modal(
            "receivables-draft-modal",
            if credit {
                "New credit note"
            } else {
                "New invoice"
            },
            "A draft has no number and posts nothing until you issue it.",
            "cancel_create",
            ModalWidth::Wide,
            &self.flash,
            form,
        )
    }

    async fn event(
        &mut self,
        scope: &Scope,
        credential: &str,
        v: &serde_json::Value,
    ) -> Result<LiveBody> {
        let mut redirect = None;
        match v["type"].as_str().ok_or(Error::Invalid)? {
            "clear-flash" => self.flash.dismiss(v),
            "filter" => self.list.apply_filters(&params::form_values(v, "filters")),
            "page" => self.list.put_page(v["page"].as_str()),
            "grid" => {
                if let ColumnsOutcome::Sort(column) = self.columns.handle(&params::string_values(v))
                {
                    self.list.next_sort(&column);
                }
            }
            kind @ ("show_create_invoice" | "show_create_credit_note") => {
                if self.pages.allowed(scope, MANAGE_DOCUMENTS).await? {
                    let kind = if kind == "show_create_invoice" {
                        "invoice"
                    } else {
                        "credit_note"
                    };
                    let mut values = HashMap::new();
                    let mut open = self.series.iter().filter(|s| s.kind == kind && s.active);
                    if let (Some(only), None) = (open.next(), open.next()) {
                        values.insert("series_id".into(), only.id.to_string());
                    }
                    self.create = Some(Draft {
                        kind,
                        values,
                        errors: Errors::default(),
                    });
                } else {
                    self.flash.set(StatusKind::Error, super::DENIED);
                }
            }
            "cancel_create" => self.create = None,
            "create_draft" => {
                let kind = self.create.as_ref().map_or("invoice", |d| d.kind);
                let form = params::form_values(v, "draft");
                let mut input = json!({"kind": kind});
                let mut local = Errors::default();
                for key in ["series_id", "customer_id", "credited_document_id"] {
                    let text = f::posted(&form, key);
                    match text.parse::<i64>() {
                        Ok(id) if id > 0 => input[key] = json!(id),
                        _ if text.is_empty() && key != "series_id" && key != "customer_id" => {}
                        _ => local
                            .0
                            .entry(key.into())
                            .or_default()
                            .push("can't be blank".into()),
                    }
                }
                for key in ["issue_date", "due_date", "description"] {
                    f::put(&mut input, key, f::posted(&form, key));
                }
                if kind == "credit_note" && input.get("credited_document_id").is_none() {
                    local
                        .0
                        .entry("credited_document_id".into())
                        .or_default()
                        .push("can't be blank".into());
                }
                let draft_kind = if kind == "credit_note" {
                    "credit_note"
                } else {
                    "invoice"
                };
                if !local.is_empty() {
                    self.create = Some(Draft {
                        kind: draft_kind,
                        values: form,
                        errors: local,
                    });
                } else {
                    match self
                        .pages
                        .write(
                            scope,
                            credential,
                            "create_draft",
                            "finance.receivables.draft.create",
                            input,
                        )
                        .await?
                    {
                        Written::Done(record) => {
                            self.create = None;
                            self.flash.set(
                                StatusKind::Success,
                                "Draft saved. Add its lines, then issue it.",
                            );
                            if let Some(id) = record["id"].as_i64() {
                                redirect = Some(format!("{INVOICES}/{id}"));
                            }
                            self.load(scope).await?;
                        }
                        Written::Invalid(fields) => {
                            self.create = Some(Draft {
                                kind: draft_kind,
                                values: form,
                                errors: Errors(fields),
                            });
                        }
                        Written::Refused(message) => {
                            self.create = None;
                            self.flash.set(
                                StatusKind::Error,
                                super::worded(
                                    message,
                                    "You do not have permission to start documents.",
                                ),
                            );
                        }
                    }
                }
            }
            _ => return Err(Error::Invalid),
        }
        let flash = redirect.is_some().then(|| self.flash.0.take()).flatten();
        Ok(LiveBody {
            html: self.body()?,
            query: self.list.to_params(),
            redirect,
            flash,
            ..LiveBody::default()
        })
    }
}

impl LiveState<Scope, Error> for Documents {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh => self.load(r.scope).await?,
                LiveInput::Params(values) => {
                    self.list =
                        ListState::parse(values, SPEC.clone()).map_err(|_| Error::Internal)?;
                }
                LiveInput::Event(v) => return self.event(r.scope, r.credential, v).await,
            }
            Ok(LiveBody {
                html: self.body()?,
                query: self.list.to_params(),
                ..LiveBody::default()
            })
        })
    }
}
