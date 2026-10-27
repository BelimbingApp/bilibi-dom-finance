//! Journal entries: every posted entry of the company's book, oldest first, a
//! hundred at a time. An entry is never edited; its page offers a reversal.
//! The date control is the shared period filter. Entry dates are civil dates,
//! both inclusive: a preset follows the display clock, and `Period::bounds`
//! is not used, because an entry date is not a timestamp. With no date in the
//! address every entry is listed, as before the filter: the shared default of
//! the last 30 days is not applied, because the trial balance and the account
//! pages open this list by account alone. That state is drawn as "All dates"
//! and an applied period offers an "All dates" button to return to it.
//! TODO(platform): an "All dates" preset on `PeriodFilter` would replace both.
use super::{
    event_type, figure, no_company, render, shell, source_label, unavailable, view, who, Flash,
    Pages, Read,
};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_datetime::{filter::Outcome, Clock, Mode, Period, PeriodFilter, Preset};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    list_state::FILTERS_FORM,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Badge, BadgeKind, BuiltinColumn, Button, Card, Cell, Choice, ColumnsOutcome,
    EmptyState, Field, Filter, FilterToolbar, FlexRow, FlexTable, Header, Html, InputKind,
    ListColumns, ListSpec, ListState, PageVariant, RecordLink, TableEmpty, ToolbarControl,
};
use jiff::{civil::Date, Timestamp};
use rust_decimal::Decimal;
use serde_json::json;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};

pub(super) const INDEX: &str = "/finance/ledger/entries";
const LIMIT: usize = 100;
const FILTER_ID: &str = "ledger-entries-period";
const ALL_DATES: &str = "all";

fn clear_event() -> String {
    format!("period_clear:{FILTER_ID}")
}

static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        page_sizes: vec![100],
        default_page_size: 100,
        filters: BTreeMap::from([
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

/// The first and last day a shared date field can edit. An open end is drawn
/// as this day; it is never written to the address or sent to the ledger.
fn earliest() -> Date {
    jiff::civil::date(1, 1, 1)
}
fn latest() -> Date {
    jiff::civil::date(9999, 12, 31)
}

/// Inclusive entry-date window. Missing ends are unbounded.
#[derive(Clone, Copy)]
struct Window {
    from: Option<Date>,
    to: Option<Date>,
    /// `None` when the address did not name a period or a closed custom range.
    preset: Option<Preset>,
}

impl Window {
    /// Reads the address. A named preset keeps the dates the address carries
    /// (the day they were applied on); `sync` moves them once the clock is
    /// known, so a reload on the same day keeps its place in the list.
    fn parse(query: &HashMap<String, String>) -> Self {
        let date = |key| query.get(key).and_then(|value| value.parse::<Date>().ok());
        let (from, to) = (date("from"), date("to"));
        let closed = matches!((from, to), (Some(from), Some(to)) if from <= to);
        // Only a named preset is read here; its fallback day never shows,
        // because `sync` resolves it on the display clock.
        let named = query
            .contains_key("period")
            .then(|| Period::parse(|key| query.get(key).cloned(), earliest()).preset);
        if let Some(preset) = named.filter(|preset| *preset != Preset::Custom) {
            return Self {
                from: closed.then_some(from).flatten(),
                to: closed.then_some(to).flatten(),
                preset: Some(preset),
            };
        }
        // A from/to pair is the custom range it applied.
        let preset = closed.then_some(Preset::Custom);
        Self { from, to, preset }
    }

    /// The preset's days on this clock, or `None` for a custom range.
    fn resolved(&self, clock: &Clock) -> Option<Period> {
        let preset = self.preset.filter(|preset| *preset != Preset::Custom)?;
        Some(Period::parse(
            |key| (key == "period").then(|| preset.as_str().to_owned()),
            clock.today,
        ))
    }

    /// Takes an applied period. A date field's own limit is an open end.
    fn apply(&mut self, period: Period) {
        self.from = (period.from != earliest()).then_some(period.from);
        self.to = (period.to != latest()).then_some(period.to);
        self.preset = (self.from.is_some() && self.to.is_some()).then_some(period.preset);
    }

    fn clear(&mut self) {
        *self = Self {
            from: None,
            to: None,
            preset: None,
        };
    }

    /// No date at all: every entry is listed.
    fn is_all(&self) -> bool {
        self.from.is_none() && self.to.is_none() && self.preset.is_none()
    }

    /// What the control shows. With every entry listed it holds the shared
    /// default range, which only seeds the custom range dialog. A half-open
    /// window draws its open end as the date field's own limit.
    fn widget(&self, clock: &Clock) -> Period {
        if self.is_all() {
            return Period::parse(|_| None, clock.today);
        }
        Period {
            preset: self.preset.unwrap_or(Preset::Custom),
            from: self.from.unwrap_or_else(earliest),
            to: self.to.unwrap_or_else(latest),
        }
    }

    fn params(&self) -> HashMap<String, String> {
        let mut query = HashMap::new();
        if let Some(preset) = self.preset {
            query.insert("period".into(), preset.as_str().to_owned());
        }
        if let Some(date) = self.from {
            query.insert("from".into(), date.to_string());
        }
        if let Some(date) = self.to {
            query.insert("to".into(), date.to_string());
        }
        query
    }
}

pub(super) struct Entries {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    flash: Flash,
    framed: bool,
    window: Window,
    filter: PeriodFilter,
    browser_zone: Option<String>,
    timezone_request: String,
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
        let window = Window::parse(query);
        // The clock is provisional until the page reads the display policy.
        let clock = Clock {
            timezone: "Etc/UTC".into(),
            today: Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date(),
        };
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("ledger-entries-table", columns)
                .map_err(|_| Error::Internal)?,
            flash: Flash::default(),
            framed,
            filter: PeriodFilter::new(FILTER_ID, window.widget(&clock), &clock),
            window,
            browser_zone: None,
            timezone_request: Timestamp::now().to_string(),
        })
    }

    fn filter_value(&self, key: &str) -> &str {
        self.list.filters.get(key).map_or("", String::as_str)
    }

    fn query(&self) -> HashMap<String, String> {
        let mut query = self.list.to_params();
        query.extend(self.window.params());
        query
    }

    /// Resolves a named preset on the display clock. Custom dates and an
    /// open end stay as the address gave them. A preset that moved to another
    /// day (the address was kept overnight) starts the list again. While the
    /// clock is still provisional (local mode before the browser reports its
    /// zone), the address's own days stand so the place in the list is kept.
    fn sync(&mut self, clock: &Clock, settled: bool) {
        if let Some(period) = self.window.resolved(clock) {
            let moved = self.window.from != Some(period.from) || self.window.to != Some(period.to);
            if moved && (settled || self.window.from.is_none()) {
                if self.window.from.is_some() {
                    self.list.filters.insert("after".into(), String::new());
                }
                self.window.apply(period);
            }
        }
        self.filter.set(self.window.widget(clock), clock);
    }

    /// The display clock, and whether it is settled: in local mode the
    /// calendar day is the browser's, which the hook reports after mount.
    async fn clock(&self, scope: &Scope) -> Result<(Clock, bool)> {
        let display = self.pages.display(scope).await?;
        let settled = display.mode != Mode::Local || self.browser_zone.is_some();
        Ok((
            display.clock(self.browser_zone.as_deref(), Timestamp::now()),
            settled,
        ))
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let (clock, settled) = self.clock(scope).await?;
        self.sync(&clock, settled);
        let mut body = Html::EMPTY;
        let mut shown = 0;
        if Pages::company(scope).is_none() {
            body.push(&no_company("ledger-entries-no-company")?);
        } else {
            match self.pages.book(scope).await? {
                Err(refused) => body.push(&unavailable(
                    "ledger-entries-refused",
                    "journal entries",
                    &refused,
                )?),
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
        Ok(LiveBody {
            html: shell("ledger-entries", PageVariant::List, &self.flash, page)?,
            query: self.query(),
            rows: shown,
            ..LiveBody::default()
        })
    }

    async fn table(&mut self, scope: &Scope, units: u8, body: &mut Html) -> Result<usize> {
        let chart = self.pages.chart(scope).await?.unwrap_or_default();
        let account: Option<i64> = self.filter_value("account").parse().ok();
        let mut input = json!({"page_size": LIMIT, "exclude_closing": false});
        if let Some(date) = self.window.from {
            input["from"] = json!(date.to_string());
        }
        if let Some(date) = self.window.to {
            input["to"] = json!(date.to_string());
        }
        if let Some(account) = account {
            input["account_id"] = json!(account);
        }
        if let Ok(pin) = self.filter_value("through").parse::<i64>() {
            input["through_entry_number"] = json!(pin);
        }
        if let Some((date, id)) = self.filter_value("after").split_once(':') {
            if let Ok(id) = id.parse::<i64>() {
                input["after_date"] = json!(date);
                input["after_id"] = json!(id);
            }
        }

        let form = self.list.filters_form();
        let mut options = vec![Choice::new("All accounts", "")];
        options.extend(
            view::by_name(chart.iter())
                .iter()
                .map(|a| Choice::new(view::account_label(a), a.id.to_string())),
        );
        let mut toolbar = FilterToolbar::new(
            "ledger-entries-filters",
            vec![ToolbarControl::select(
                "ledger-entries-account",
                "account",
                "Account",
                &form["account"],
                options,
            )],
        );
        toolbar.class = "mb-0 min-w-0 flex-1";
        let mut clear = Html::EMPTY;
        let period = if self.window.is_all() && !self.filter.is_open() {
            let mut options = vec![Choice::new("All dates", ALL_DATES)];
            options.extend(
                Preset::ALL
                    .iter()
                    .map(|(label, preset)| Choice::new(*label, preset.as_str())),
            );
            render(AllDates {
                id: format!("{FILTER_ID}-all"),
                event: self.filter.select_event(),
                field: render(
                    Field::in_form("period_filter", "period")
                        .id(format!("{FILTER_ID}-period"))
                        .kind(InputKind::Select)
                        .label(self.filter.label.clone())
                        .label_class("sr-only")
                        .wrapper_class("mb-0")
                        .value(ALL_DATES)
                        .options(options),
                )?,
            })?
        } else {
            if !self.window.is_all() {
                clear = render(Button {
                    id: "ledger-entries-period-clear",
                    button_type: "button",
                    event: &clear_event(),
                    body: Html::text("All dates"),
                    ..Button::default()
                })?;
            }
            render(self.filter.clone())?
        };
        body.push(&render(FilterRegion {
            timezone_request: &self.timezone_request,
            toolbar: render(toolbar)?,
            period,
            clear,
        })?);
        if !self.filter_value("through").is_empty() {
            body.push(&render(Notice {
                text: format!(
                    "These are the entries through number {}, as the trial balance counted them. Later entries are left out.",
                    self.filter_value("through")
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
            "No entries match. Change the period or the account, or post an entry.",
        )));
        let mut card = render(table)?;
        if records.len() == LIMIT {
            if let Some(last) = records.last() {
                let mut later = self.list.clone();
                later
                    .filters
                    .insert("after".into(), format!("{}:{}", last.entry_date, last.id));
                let mut query = later.to_params();
                query.extend(self.window.params());
                // Later entries are read at the same point in the ledger, so
                // the pages join up even while new entries arrive.
                card.push(&render(LaterLink {
                    href: params::address(INDEX, &query),
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
    source = r#"<div id="ledger-entries-period-region" data-hook="BrowserTimeZone" data-timezone-request="{{ timezone_request }}" class="mb-3 flex flex-wrap items-start gap-3">{{ toolbar }}{{ period }}{{ clear }}</div>"#,
    ext = "html"
)]
struct FilterRegion<'a> {
    timezone_request: &'a str,
    toolbar: Html,
    period: Html,
    clear: Html,
}

#[derive(Template)]
#[template(
    source = r#"<form id="{{ id }}" data-event="{{ event }}" data-change class="ui-compact-controls w-full sm:w-fit sm:min-w-36">{{ field }}</form>"#,
    ext = "html"
)]
struct AllDates {
    id: String,
    event: String,
    field: Html,
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
                    self.window = Window::parse(values);
                }
                LiveInput::Event(value) => {
                    let kind = event_type(value)?;
                    let (clock, _) = self.clock(r.scope).await?;
                    if kind == clear_event() {
                        self.window.clear();
                        self.list.filters.insert("after".into(), String::new());
                    } else if let Some(outcome) = self.filter.event(kind, value, &clock) {
                        if let Outcome::Applied(period) = outcome {
                            self.window.apply(period);
                            self.filter.set(period, &clock);
                            // A new period starts from the first matching entry.
                            self.list.filters.insert("after".into(), String::new());
                        }
                    } else {
                        match kind {
                            "filters" => {
                                let mut posted = params::form_values(value, FILTERS_FORM);
                                // A new account starts from the first entry again.
                                posted.insert("after".into(), String::new());
                                self.list.apply_filters(&posted);
                            }
                            "grid" => {
                                if let ColumnsOutcome::Sort(_) =
                                    self.columns.handle(&params::string_values(value))
                                {
                                }
                            }
                            "browser_timezone" => {
                                self.browser_zone = value["timezone"]
                                    .as_str()
                                    .filter(|zone| bilibi_datetime::valid_timezone(zone))
                                    .map(str::to_owned);
                                self.timezone_request.clear();
                            }
                            "clear-flash" => self.flash.dismiss(value),
                            _ => return Err(Error::Invalid),
                        }
                    }
                }
            }
            self.view(r.scope).await
        })
    }
}
