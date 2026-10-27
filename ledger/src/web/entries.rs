//! Journal entries: every posted entry of the company's book, oldest first, a
//! hundred at a time. An entry is never edited; its page offers a reversal.
use super::{
    event_type, figure, no_company, render, shell, source_label, unavailable, view, who, Flash,
    Pages, Read,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    list_state::FILTERS_FORM,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Badge, BadgeKind, BuiltinColumn, Card, Cell, Choice, ColumnsOutcome, EmptyState,
    Filter, FilterToolbar, FlexRow, FlexTable, Header, Html, ListColumns, ListSpec, ListState,
    PageVariant, RecordLink, TableEmpty, ToolbarControl,
};
use rust_decimal::Decimal;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

pub(super) const INDEX: &str = "/finance/ledger/entries";
const LIMIT: usize = 100;

static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        page_sizes: vec![100],
        default_page_size: 100,
        filters: BTreeMap::from([
            ("from".into(), Filter::string("")),
            ("to".into(), Filter::string("")),
            ("account".into(), Filter::string("")),
            ("after".into(), Filter::string("")),
            ("through".into(), Filter::string("")),
        ]),
        omit_defaults: true,
        ..ListSpec::default()
    }
    .shared()
    .expect("journal entries list spec")
});

pub(super) struct Entries {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    flash: Flash,
    framed: bool,
}

impl Entries {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = vec![
            BuiltinColumn::new("number", "Number"),
            BuiltinColumn::new("date", "Date"),
            BuiltinColumn::new("description", "Description"),
            BuiltinColumn::new("reference", "Reference"),
            BuiltinColumn::new("source", "Source"),
            BuiltinColumn::new("amount", "Amount").right(),
            BuiltinColumn::new("account_side", "On the chosen account").right(),
            BuiltinColumn::new("status", "Status"),
            BuiltinColumn::new("by", "Posted by"),
        ];
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("ledger-entries-table", columns)
                .map_err(|_| Error::Internal)?,
            flash: Flash::default(),
            framed,
        })
    }

    fn filter(&self, key: &str) -> &str {
        self.list.filters.get(key).map_or("", String::as_str)
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let mut body = Html::EMPTY;
        let mut shown = 0;
        if Pages::company(scope).is_none() {
            body.push(&no_company("ledger-entries-no-company")?);
        } else {
            match self.pages.book(scope).await? {
                Err(refused) => body.push(&unavailable("ledger-entries-refused", "journal entries", &refused)?),
                Ok(state) => match state.book {
                    None => body.push(&render(
                        EmptyState::missing(
                            "The book is not open yet",
                            "Entries appear here once the company's book is open and something is posted.",
                        )
                        .id("ledger-entries-no-book"),
                    )?),
                    Some(book) => shown = self.table(scope, book.minor_units, &mut body).await?,
                },
            }
        }
        let mut page = render(Header {
            title: Html::text("Journal entries"),
            ..Header::default()
        })?;
        page.push(&body);
        let query = self.list.to_params();
        Ok(LiveBody {
            html: shell("ledger-entries", PageVariant::List, &self.flash, page)?,
            query,
            rows: shown,
            ..LiveBody::default()
        })
    }

    async fn table(&mut self, scope: &Scope, units: u8, body: &mut Html) -> Result<usize> {
        let chart = self.pages.chart(scope).await?.unwrap_or_default();
        let account: Option<i64> = self.filter("account").parse().ok();
        let mut input = json!({"page_size": LIMIT, "exclude_closing": false});
        for (key, field) in [("from", "from"), ("to", "to")] {
            if !self.filter(key).is_empty() {
                input[field] = json!(self.filter(key));
            }
        }
        if let Some(account) = account {
            input["account_id"] = json!(account);
        }
        if let Ok(pin) = self.filter("through").parse::<i64>() {
            input["through_entry_number"] = json!(pin);
        }
        if let Some((date, id)) = self.filter("after").split_once(':') {
            if let Ok(id) = id.parse::<i64>() {
                input["after_date"] = json!(date);
                input["after_id"] = json!(id);
            }
        }

        let form = self.list.filters_form();
        let mut options = vec![Choice::new("All accounts", "")];
        options.extend(
            chart
                .iter()
                .map(|a| Choice::new(format!("{} {}", a.code, a.name), a.id.to_string())),
        );
        body.push(&render(FilterToolbar::new(
            "ledger-entries-filters",
            vec![
                ToolbarControl::date("ledger-entries-from", "from", "From date", &form["from"]),
                ToolbarControl::date("ledger-entries-to", "to", "To date", &form["to"]),
                ToolbarControl::select(
                    "ledger-entries-account",
                    "account",
                    "Account",
                    &form["account"],
                    options,
                ),
            ],
        ))?);
        if !self.filter("through").is_empty() {
            body.push(&render(Notice {
                text: format!(
                    "These are the entries through number {}, as the trial balance counted them. Later entries are left out.",
                    self.filter("through")
                ),
            })?);
        }

        let records: Vec<view::Entry> = match self
            .pages
            .read(scope, "finance.ledger.entry.list", input)
            .await?
        {
            Read::Found(value) => view::parse(value["records"].clone()).unwrap_or_default(),
            other => {
                body.push(&unavailable(
                    "ledger-entries-refused",
                    "journal entries",
                    &other,
                )?);
                return Ok(0);
            }
        };
        self.columns.load(&records, |e| e.id.to_string());
        let mut rows = Vec::new();
        for entry in &records {
            let total: Decimal = entry.lines.iter().map(|l| view::decimal(&l.debit)).sum();
            let on_account = account.map(|id| {
                let debit: Decimal = entry
                    .lines
                    .iter()
                    .filter(|l| l.account_id == id)
                    .map(|l| view::decimal(&l.debit))
                    .sum();
                let credit: Decimal = entry
                    .lines
                    .iter()
                    .filter(|l| l.account_id == id)
                    .map(|l| view::decimal(&l.credit))
                    .sum();
                if debit >= credit {
                    format!(
                        "{} debit",
                        view::amount(&(debit - credit).to_string(), units)
                    )
                } else {
                    format!(
                        "{} credit",
                        view::amount(&(credit - debit).to_string(), units)
                    )
                }
            });
            let mut row = FlexRow::new(entry.id.to_string())
                .slot(
                    "number",
                    render(RecordLink {
                        workspace: self.framed,
                        kind: crate::MODULE,
                        record_id: &entry.id.to_string(),
                        href: &format!("{INDEX}/{}", entry.id),
                        class: "font-medium text-ink-strong hover:underline",
                        body: Html::text(&entry.entry_number.to_string()),
                    })?,
                )
                .cell("date", Cell::text(&entry.entry_date))
                .cell("description", Cell::text(&entry.description))
                .cell(
                    "reference",
                    Cell::text(entry.reference.as_deref().unwrap_or("—")),
                )
                .cell(
                    "source",
                    Cell::text(
                        entry
                            .owner
                            .as_ref()
                            .map_or("Manual".to_owned(), |o| source_label(&o.module)),
                    ),
                )
                .slot("amount", figure(&view::amount(&total.to_string(), units))?)
                .slot(
                    "account_side",
                    figure(&on_account.unwrap_or_else(|| "—".into()))?,
                )
                .slot("status", entry_status(entry)?)
                .cell(
                    "by",
                    Cell::text(who(
                        entry.posted_by_user_id,
                        entry.posted_by_impersonator_id,
                    )),
                );
            row.dom_id = Some(format!("ledger-entry-{}", entry.id));
            rows.push(row);
        }
        let mut table = FlexTable::new("ledger-entries-table", self.columns.flex_columns(), rows);
        table.caption = "Journal entries";
        table.framed = false;
        table.zoom = self.columns.zoom;
        table.suggestions = self.columns.flex_suggestions();
        table.add_query = &self.columns.add_query;
        table.empty = Some(TableEmpty::Content(Html::text(
            "No entries match. Change the dates or the account, or post an entry.",
        )));
        let mut card = render(table)?;
        if records.len() == LIMIT {
            if let Some(last) = records.last() {
                let mut later = self.list.clone();
                later
                    .filters
                    .insert("after".into(), format!("{}:{}", last.entry_date, last.id));
                // Later entries are read at the same point in the ledger, so
                // the pages join up even while new entries arrive.
                card.push(&render(LaterLink {
                    href: params::address(INDEX, &later.to_params()),
                })?);
            }
        }
        body.push(&render(Card {
            id: "ledger-entries-card",
            inner_class: "p-0",
            body: card,
            ..Card::default()
        })?);
        Ok(records.len())
    }
}

pub(super) fn entry_status(entry: &view::Entry) -> Result<Html> {
    if entry.reversed_by_entry_id.is_some() {
        render(Badge::status(BadgeKind::Warning, "Reversed"))
    } else if entry.reverses_entry_id.is_some() {
        render(Badge::status(BadgeKind::Neutral, "Reversal"))
    } else {
        render(Badge::status(BadgeKind::Success, "Posted"))
    }
}

#[derive(Template)]
#[template(
    source = r#"<div class="p-3 text-sm"><a id="ledger-entries-later" href="{{ href }}" data-patch>Later entries</a></div>"#,
    ext = "html"
)]
struct LaterLink {
    href: String,
}

#[derive(Template)]
#[template(
    source = r#"<p id="ledger-entries-pin" class="mb-3 text-sm text-ink-muted">{{ text }}</p>"#,
    ext = "html"
)]
struct Notice {
    text: String,
}

impl LiveState<Scope, Error> for Entries {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh => {}
                LiveInput::Params(values) => {
                    self.list =
                        ListState::parse(values, SPEC.clone()).map_err(|_| Error::Internal)?;
                }
                LiveInput::Event(value) => match event_type(value)? {
                    "filters" => {
                        let mut posted = params::form_values(value, FILTERS_FORM);
                        // A new filter starts from the first entry again.
                        posted.insert("after".into(), String::new());
                        self.list.apply_filters(&posted);
                    }
                    "grid" => {
                        if let ColumnsOutcome::Sort(_) =
                            self.columns.handle(&params::string_values(value))
                        {
                        }
                    }
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                },
            }
            self.view(r.scope).await
        })
    }
}
