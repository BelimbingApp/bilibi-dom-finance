//! Company books: every company the person may work in, with the state of its
//! book and fiscal calendar, side by side. It is an overview of Finance's own
//! facts, not a consolidation: each row stands alone, in its own currency. It
//! reads each company through the same contracts an agent calls (`book.get`,
//! `fiscal_year.list`), which prove the reach again, and it never switches the
//! session's current company.
use super::{event_type, no_company, render, shell, view, Company, Flash, Pages, Read};
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Badge, BadgeKind, BuiltinColumn, Card, Cell, FlexRow, FlexTable, Header, Html,
    ListColumns, PageVariant, TableEmpty,
};
use jiff::civil::Date;

pub(super) struct Books {
    pages: Pages,
    flash: Flash,
    columns: Option<ListColumns>,
}

/// What a company's book answered.
enum Book {
    Open(view::BookRow),
    NotOpen,
    /// The person may work in the company but the ledger refused the read.
    NoAccess,
    Unavailable,
}

/// The state of a company's fiscal calendar, as the books page words it.
struct Calendar {
    latest: Option<(String, String, bool)>,
    earliest_open: Option<(String, String)>,
    past_due: bool,
}

impl Calendar {
    fn of(years: &[view::Year], today: Date) -> Self {
        let latest = years.last().map(|y| {
            (
                y.start_date.clone(),
                y.end_date.clone(),
                y.status == "closed",
            )
        });
        let earliest_open = years
            .iter()
            .flat_map(|y| y.periods.iter())
            .find(|p| p.status == "open")
            .map(|p| (p.start_date.clone(), p.end_date.clone()));
        let past_due = earliest_open
            .as_ref()
            .and_then(|(_, end)| end.parse::<Date>().ok())
            .is_some_and(|end| end < today);
        Self {
            latest,
            earliest_open,
            past_due,
        }
    }
}

/// The company's name in a row, marked when it is the session's current one.
#[derive(askama::Template)]
#[template(
    source = r#"<span class="font-medium text-ink-strong">{{ name }}</span>{% if current %} <span class="text-ink-muted">current company</span>{% endif %}"#,
    ext = "html"
)]
struct CompanyCell<'a> {
    name: &'a str,
    current: bool,
}

/// The way into one company's fiscal years from its row. It filters the fiscal
/// years page to the company; it does not switch the current company.
#[derive(askama::Template)]
#[template(
    source = r#"<a id="ledger-books-years-{{ id }}" href="/finance/ledger/fiscal-years?company={{ id }}" aria-label="Fiscal years of {{ name }}" class="text-link hover:underline">Fiscal years</a>"#,
    ext = "html"
)]
struct YearsLink<'a> {
    id: i64,
    name: &'a str,
}

#[derive(askama::Template)]
#[template(
    source = r#"<p id="ledger-books-note" class="mt-3 text-xs text-ink-muted">Companies you cannot work in are not listed. A row shows one company in its own currency; figures are never added across companies. Choosing a company here does not make it your current company: use the top bar for that.</p>"#,
    ext = "html"
)]
struct Note;

/// Just the id of a row, for the column manager.
struct Line(String);

impl Books {
    pub(super) fn new(pages: Pages) -> Self {
        Self {
            pages,
            flash: Flash::default(),
            columns: None,
        }
    }

    /// One company's row. A company the person works in whose book the ledger
    /// will not read says so, apart from one whose book is simply not open.
    async fn row(
        &self,
        scope: &Scope,
        company: &Company,
        current: Option<i64>,
        today: Date,
    ) -> Result<FlexRow> {
        let book = match self.pages.book_in(scope, company.id).await? {
            Ok(state) => match state.book {
                Some(book) => Book::Open(book),
                None => Book::NotOpen,
            },
            Err(Read::Denied) => Book::NoAccess,
            Err(_) => Book::Unavailable,
        };
        let mut row = FlexRow::new(company.id.to_string()).slot(
            "company",
            render(CompanyCell {
                name: &company.name,
                current: Some(company.id) == current,
            })?,
        );
        row.dom_id = Some(format!("ledger-books-{}", company.id));
        let dash = || Cell::text("—");
        match &book {
            Book::Open(book) => {
                row = row
                    .slot("book", render(Badge::status(BadgeKind::Success, "Open"))?)
                    .cell(
                        "currency",
                        Cell::text(format!(
                            "{} · {} decimal places",
                            book.currency_code, book.minor_units
                        )),
                    );
                match self.pages.years_in(scope, company.id).await? {
                    Ok(years) if years.is_empty() => {
                        row = row
                            .slot(
                                "year",
                                render(Badge::status(BadgeKind::Warning, "No fiscal year yet"))?,
                            )
                            .cell("open", dash());
                    }
                    Ok(years) => {
                        let calendar = Calendar::of(&years, today);
                        row = match calendar.latest {
                            Some((start, end, closed)) => row.cell(
                                "year",
                                Cell::text(format!(
                                    "{start} to {end} ({})",
                                    if closed { "closed" } else { "open" }
                                )),
                            ),
                            None => row.cell("year", dash()),
                        };
                        row = match calendar.earliest_open {
                            Some((start, end)) => {
                                let mut cell = Html::text(&format!("{start} to {end}"));
                                if calendar.past_due {
                                    cell.push(&Html::text(" "));
                                    cell.push(&render(Badge::status(
                                        BadgeKind::Warning,
                                        "Past due",
                                    ))?);
                                }
                                row.slot("open", cell)
                            }
                            None => row.cell("open", Cell::text("None open")),
                        };
                    }
                    Err(Read::Denied) => {
                        row = row
                            .cell("year", Cell::text("No access to fiscal years"))
                            .cell("open", dash());
                    }
                    Err(_) => {
                        row = row
                            .cell("year", Cell::text("Could not be read"))
                            .cell("open", dash());
                    }
                }
            }
            Book::NotOpen => {
                row = row
                    .slot(
                        "book",
                        render(Badge::status(BadgeKind::Warning, "Not open"))?,
                    )
                    .cell("currency", dash())
                    .cell("year", dash())
                    .cell("open", dash());
            }
            Book::NoAccess => {
                row = row
                    .slot(
                        "book",
                        render(Badge::status(BadgeKind::Neutral, "No access"))?,
                    )
                    .cell("currency", dash())
                    .cell("year", dash())
                    .cell("open", dash());
            }
            Book::Unavailable => {
                row = row
                    .slot(
                        "book",
                        render(Badge::status(BadgeKind::Neutral, "Could not be read"))?,
                    )
                    .cell("currency", dash())
                    .cell("year", dash())
                    .cell("open", dash());
            }
        }
        // Years can be read only where the book is open.
        if matches!(book, Book::Open(_)) {
            row = row.slot(
                "view",
                render(YearsLink {
                    id: company.id,
                    name: &company.name,
                })?,
            );
        } else {
            row = row.cell("view", dash());
        }
        Ok(row)
    }

    async fn view(&mut self, scope: &Scope) -> Result<LiveBody> {
        let companies = self.pages.companies(scope).await?;
        let today = self.pages.today(scope).await?;
        let current = Pages::company(scope);
        let mut body = Html::EMPTY;
        let mut shown = 0;
        if companies.is_empty() {
            body.push(&no_company("ledger-books-no-company")?);
        } else {
            let mut rows = Vec::new();
            for company in &companies {
                rows.push(self.row(scope, company, current, today).await?);
            }
            shown = rows.len();
            if self.columns.is_none() {
                self.columns = Some(
                    ListColumns::mount(
                        "ledger-books",
                        vec![
                            BuiltinColumn::new("company", "Company"),
                            BuiltinColumn::new("book", "Book"),
                            BuiltinColumn::new("currency", "Currency"),
                            BuiltinColumn::new("year", "Latest fiscal year"),
                            BuiltinColumn::new("open", "Earliest open period"),
                            BuiltinColumn::new("view", "See"),
                        ],
                    )
                    .map_err(|_| Error::Internal)?,
                );
            }
            let columns = self.columns.as_mut().ok_or(Error::Internal)?;
            let lines: Vec<Line> = companies.iter().map(|c| Line(c.id.to_string())).collect();
            columns.load(&lines, |line| line.0.clone());
            let mut table = FlexTable::new("ledger-books-table", columns.flex_columns(), rows);
            table.caption = "Company books";
            table.framed = false;
            table.zoom = columns.zoom;
            table.suggestions = columns.flex_suggestions();
            table.add_query = &columns.add_query;
            table.empty = Some(TableEmpty::Content(Html::text("No company to show.")));
            body.push(&render(Card {
                id: "ledger-books-card",
                inner_class: "p-0",
                body: render(table)?,
                ..Card::default()
            })?);
            body.push(&render(Note)?);
        }
        let mut page = render(Header {
            title: Html::text("Company books"),
            subtitle: "The book and fiscal calendar of each company you can work in",
            title_actions: super::menu_pin(
                "ledger-books-pin",
                "nav-finance-ledger-books",
                "Company books",
            )?,
            ..Header::default()
        })?;
        page.push(&body);
        Ok(LiveBody {
            html: shell("ledger-books", PageVariant::List, &self.flash, page)?,
            rows: shown,
            ..LiveBody::default()
        })
    }
}

impl LiveState<Scope, Error> for Books {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Event(value) = r.input {
                match event_type(value)? {
                    "grid" => {
                        if let Some(columns) = &mut self.columns {
                            columns.handle(&params::string_values(value));
                        }
                    }
                    "clear-flash" => self.flash.dismiss(value),
                    _ => return Err(Error::Invalid),
                }
            }
            self.view(r.scope).await
        })
    }
}
