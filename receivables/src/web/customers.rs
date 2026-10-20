//! The customers list and its Add customer dialog. Reads
//! `finance.receivables.customer.list`; Add calls `customer.create`.
use super::{
    format::{self as f, Errors},
    render, Flash, Pages, Written, CUSTOMERS,
};
use crate::Customer;
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
        sortable: BTreeMap::from([("code".into(), SortDir::Asc), ("name".into(), SortDir::Asc)]),
        default_sort: Some("code".into()),
        page_sizes: SIZES.to_vec(),
        filters: [(
            "status".into(),
            Filter::one_of(&["", "active", "inactive"], ""),
        )]
        .into(),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("customers list spec")
});

/// The Add customer form's fields, in the order a clerk meets them.
/// `(key, label, kind, maximum length, required, hint)`
pub(super) const FIELDS: &[(&str, &str, InputKind, &str, bool, &str)] = &[
    (
        "code",
        "Code",
        InputKind::Text,
        "50",
        true,
        "A short name for this customer. It cannot be changed later.",
    ),
    ("name", "Name", InputKind::Text, "200", true, ""),
    ("email", "Email", InputKind::Email, "255", false, ""),
    ("phone", "Phone", InputKind::Tel, "50", false, ""),
    (
        "payment_terms_days",
        "Payment terms (days)",
        InputKind::Number,
        "4",
        false,
        "Days after the invoice date that payment is due. Leave empty for due on issue.",
    ),
    ("tax_identifier", "Tax ID", InputKind::Text, "50", false, ""),
    (
        "registration_identifier",
        "Registration number",
        InputKind::Text,
        "50",
        false,
        "",
    ),
    ("line1", "Address line 1", InputKind::Text, "255", false, ""),
    ("line2", "Address line 2", InputKind::Text, "255", false, ""),
    ("line3", "Address line 3", InputKind::Text, "255", false, ""),
    ("locality", "City", InputKind::Text, "100", false, ""),
    ("postcode", "Postcode", InputKind::Text, "20", false, ""),
    (
        "admin1_code",
        "State or region",
        InputKind::Text,
        "20",
        false,
        "",
    ),
    (
        "country_iso",
        "Country code",
        InputKind::Text,
        "2",
        false,
        "Two capital letters, such as MY.",
    ),
];

pub(super) struct Customers {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    all: Vec<Customer>,
    cut: bool,
    can_add: bool,
    create: Option<(HashMap<String, String>, Errors)>,
    flash: Flash,
    framed: bool,
}

impl Customers {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("code", "Code").sortable("code"),
            BuiltinColumn::new("name", "Name").sortable("name"),
            BuiltinColumn::new("email", "Email"),
            BuiltinColumn::new("phone", "Phone"),
            BuiltinColumn::new("terms", "Terms").right(),
            BuiltinColumn::new("status", "Status"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("receivables-customers-table", columns)
                .map_err(|_| Error::Internal)?,
            all: vec![],
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
                "finance.receivables.customer.list",
                json!({}),
                ("after_code", "next_after_code"),
            )
            .await?;
        self.can_add = self.pages.allowed(scope, crate::MANAGE_CUSTOMERS).await?;
        Ok(())
    }

    /// The rows the filters, search and sort leave, and the page asked for.
    fn shown(&mut self) -> (Vec<&Customer>, u64) {
        let search = self.list.search.trim().to_lowercase();
        let status = self.list.filters.get("status").cloned().unwrap_or_default();
        let mut rows: Vec<&Customer> = self
            .all
            .iter()
            .filter(|c| match status.as_str() {
                "active" => c.active,
                "inactive" => !c.active,
                _ => true,
            })
            .filter(|c| {
                search.is_empty()
                    || [
                        Some(&c.code),
                        Some(&c.name),
                        c.email.as_ref(),
                        c.phone.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .any(|t| t.to_lowercase().contains(&search))
            })
            .collect();
        match self.list.sort_by.as_deref() {
            Some("name") => rows.sort_by_key(|c| c.name.to_lowercase()),
            _ => rows.sort_by(|a, b| a.code.cmp(&b.code)),
        }
        if self.list.sort_dir == Some(SortDir::Desc) {
            rows.reverse();
        }
        let total = rows.len() as u64;
        let size = self.list.page_size.max(1);
        self.list.page = self.list.page.clamp(1, total.div_ceil(size).max(1));
        let from = ((self.list.page - 1) * size) as usize;
        (
            rows.into_iter().skip(from).take(size as usize).collect(),
            total,
        )
    }

    fn body(&mut self) -> Result<String> {
        let framed = self.framed;
        let (rows, total) = self.shown();
        let shown: Vec<Customer> = rows.into_iter().cloned().collect();
        self.columns.load(&shown, |c| c.id.to_string());
        let actions = if self.can_add {
            render(CreateButton {
                noun: "Customer",
                id: "receivables-customers-add",
                event: "show_create",
                ..CreateButton::default()
            })?
        } else {
            Html::EMPTY
        };
        let mut content = render(FilterToolbar {
            event: "filter",
            ..FilterToolbar::new(
                "receivables-customers-filters",
                vec![
                    ToolbarControl::search(
                        "receivables-customers-search",
                        "search",
                        "Search customers",
                        &self.list.search,
                        "Search by code, name, email or phone…",
                    ),
                    ToolbarControl::select(
                        "receivables-customers-status",
                        "status",
                        "Filter by status",
                        &self.list.filters["status"],
                        [
                            ("All statuses", ""),
                            ("Active", "active"),
                            ("Inactive", "inactive"),
                        ]
                        .map(|(l, v)| Choice::new(l, v))
                        .to_vec(),
                    ),
                ],
            )
        })?;
        let mut flex_rows = Vec::new();
        for c in &shown {
            let mut row = FlexRow::new(c.id.to_string());
            row.dom_id = Some(format!("receivables-customer-{}", c.id));
            flex_rows.push(
                row.slot(
                    "code",
                    render(RecordLink {
                        workspace: framed,
                        kind: crate::MODULE,
                        record_id: &c.id.to_string(),
                        href: &format!("{CUSTOMERS}/{}", c.id),
                        class: "font-medium text-ink-strong hover:underline",
                        body: Html::text(&c.code),
                    })?,
                )
                .cell("name", Cell::text(&c.name))
                .cell("email", Cell::text(f::optional(&c.email)))
                .cell("phone", Cell::text(f::optional(&c.phone)))
                .slot(
                    "terms",
                    f::tabular(&format!("{} days", c.payment_terms_days))?,
                )
                .slot("status", f::active_badge(c.active)?),
            );
        }
        let mut table = FlexTable::new(
            "receivables-customers-table",
            self.columns.flex_columns(),
            flex_rows,
        )
        .sorted(self.list.sort_by.as_deref(), self.list.sort_dir);
        table.caption = "Customers";
        table.framed = false;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(render(EmptyState::missing(
            "No customers",
            if self.all.is_empty() {
                "Add the people and companies you invoice, then raise invoices to them."
            } else {
                "No customer matches the current filters."
            },
        ))?));
        content.push(&render(table)?);
        content.push(&render(Pagination {
            filters_event: "filter",
            ..Pagination::new(
                "receivables-customers-pagination",
                PageCounts::new(self.list.page, self.list.page_size, total),
                &SIZES,
            )
        })?);
        if self.cut {
            content.push(&Html::text(
                "Only the first customers are listed. Search to narrow them.",
            ));
        }
        let header = render(Header {
            title: Html::text("Customers"),
            subtitle: "Who you invoice, and on what payment terms.",
            actions,
            ..Header::default()
        })?;
        let mut body = header;
        body.push(&content);
        let modal = match &self.create {
            Some((values, errors)) => Some(self.dialog(values, errors)?),
            None => None,
        };
        f::frame(
            "receivables-customers",
            PageVariant::List,
            &self.flash,
            body,
            modal,
        )
    }

    fn dialog(&self, values: &HashMap<String, String>, errors: &Errors) -> Result<Html> {
        let labels: Vec<_> = FIELDS
            .iter()
            .map(|(key, label, ..)| {
                (
                    (*key).to_owned(),
                    format!("receivables-customer-new-{key}"),
                    (*label).to_owned(),
                )
            })
            .collect();
        let mut body = render(errors.summary("receivables-customer-new-errors", &labels))?;
        let mut grid = Html::EMPTY;
        for (key, label, kind, max, required, hint) in FIELDS {
            let mut field = Field::in_form("customer", key)
                .id(format!("receivables-customer-new-{key}"))
                .label(*label)
                .kind(*kind)
                .value(values.get(*key).cloned().unwrap_or_default())
                .errors(errors.of(key));
            field = if *kind == InputKind::Number {
                field
                    .attr("min", "0")
                    .attr("max", max_for_terms())
                    .attr("step", "1")
            } else {
                field.attr("maxlength", max)
            };
            if *required {
                field = field.required();
            }
            if !hint.is_empty() {
                field = field.hint(*hint);
            }
            grid.push(&render(field)?);
        }
        body.push(&f::two(grid)?);
        let actions = {
            let mut a = render(Button {
                id: "receivables-customer-new-save",
                button_type: "submit",
                variant: ButtonVariant::Primary,
                working: "Adding…",
                body: Html::text("Add customer"),
                ..Button::default()
            })?;
            a.push(&render(Button {
                id: "receivables-customer-new-cancel",
                button_type: "button",
                event: "cancel_create",
                body: Html::text("Cancel"),
                ..Button::default()
            })?);
            a
        };
        let form = render(Form {
            id: "receivables-customer-new-form",
            event: "create_customer",
            change: false,
            class: "",
            body,
            actions,
        })?;
        f::modal(
            "receivables-customer-new",
            "Add customer",
            "",
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
            "show_create" => {
                if self.pages.allowed(scope, crate::MANAGE_CUSTOMERS).await? {
                    self.create = Some((HashMap::new(), Errors::default()));
                } else {
                    self.flash.set(StatusKind::Error, super::DENIED);
                }
            }
            "cancel_create" => self.create = None,
            "create_customer" => {
                let form = params::form_values(v, "customer");
                let mut input = json!({});
                let mut local = Errors::default();
                for (key, _, kind, ..) in FIELDS {
                    let text = f::posted(&form, key);
                    if *kind == InputKind::Number {
                        match text.parse::<i64>() {
                            Ok(n) if !text.is_empty() => input[*key] = json!(n),
                            _ if text.is_empty() => {}
                            _ => local
                                .0
                                .entry((*key).into())
                                .or_default()
                                .push("must be a whole number of days".into()),
                        }
                    } else if *key == "country_iso" {
                        f::put(&mut input, key, &text.to_uppercase());
                    } else {
                        f::put(&mut input, key, text);
                    }
                }
                if !local.is_empty() {
                    self.create = Some((form, local));
                } else {
                    match self
                        .pages
                        .write(
                            scope,
                            credential,
                            "create_customer",
                            "finance.receivables.customer.create",
                            input,
                        )
                        .await?
                    {
                        Written::Done(record) => {
                            self.create = None;
                            self.flash.set(StatusKind::Success, "Customer added.");
                            if let Some(id) = record["id"].as_i64() {
                                redirect = Some(format!("{CUSTOMERS}/{id}"));
                            }
                            self.load(scope).await?;
                        }
                        Written::Invalid(fields) => self.create = Some((form, Errors(fields))),
                        Written::Refused(message) => {
                            self.create = None;
                            self.flash.set(
                                StatusKind::Error,
                                super::worded(
                                    message,
                                    "You do not have permission to add customers.",
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

fn max_for_terms() -> &'static str {
    "3650"
}

impl LiveState<Scope, Error> for Customers {
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
