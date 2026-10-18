//! Bilimbi Reports.Web.{BookkeepingLive,ApprovalExceptionsLive} at
//! 4abfaf829321f8540581d833752a086b21b45ba9. Ledger owns all reads and writes.
use crate::{Reports, VIEW};
use askama::Template;
use bilibi_company::organisation;
use bilibi_database::{Error, Result};
use bilibi_datetime::{
    Clock, DisplayContext, DisplayPolicy, Format, Period, PeriodFilter, Precision,
};
use bilibi_finance_ledger::{
    ApprovalExceptionQuery, LateRecordingQuery, LateStatus, LedgerError, NewFiscalYear,
    MANAGE_CONTROLLED_BOOKKEEPING, MANAGE_DUTY_SEPARATION, MANAGE_PERIODS, REVIEW_EXCEPTIONS,
    REVIEW_LATE_RECORDINGS,
};
use bilibi_locale::Locales;
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{
        LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState, MountFuture, Page, PageBody,
        PageFuture, PageRequest,
    },
    params, Alert, Badge, BadgeKind, BuiltinColumn, Button, ButtonVariant, Card, EmptyState, Field,
    Filter, FlashMap, FlexRow, FlexTable, Header, Html, InputKind, ListColumns, ListSpec,
    ListState, Modal, RowMode, SectionHeading, StatusKind, Tab, Tabs, NO_FLASH,
};
use jiff::{civil::Date, Timestamp};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc};

#[derive(Clone)]
pub(crate) struct Pages {
    pub reports: Arc<Reports>,
    pub display: Arc<DisplayPolicy>,
    pub locales: Arc<Locales>,
}

#[derive(Clone, Copy)]
enum Dialog {
    ControlledOff,
    FiscalYear,
    Separation { required: bool },
    Review { id: i64 },
}

struct State {
    pages: Pages,
    bookkeeping: bool,
    list: ListState,
    columns: Vec<ListColumns>,
    browser_zone: Option<String>,
    period: Option<PeriodFilter>,
    dialog: Option<Dialog>,
    text: String,
    dates: HashMap<String, String>,
    flash: Option<(StatusKind, String)>,
}

impl State {
    fn new(pages: Pages, operation: &str, query: &HashMap<String, String>) -> Result<Self> {
        let bookkeeping = match operation {
            "bookkeeping" => true,
            "approval-exceptions" => false,
            _ => return Err(Error::NotFound),
        };
        let mut spec = ListSpec {
            page_sizes: vec![100],
            default_page_size: 100,
            omit_defaults: true,
            ..ListSpec::default()
        };
        spec.filters.insert(
            "review".into(),
            Filter::one_of(&["all", "pending", "reviewed"], "all"),
        );
        spec.filters.insert("after".into(), Filter::string(""));
        if bookkeeping {
            spec.filters.insert(
                "status".into(),
                Filter::one_of(&["all", "approaching", "late"], "all"),
            );
        } else {
            spec.filters.insert(
                "period".into(),
                Filter::one_of(
                    &[
                        "today",
                        "this_month",
                        "last_7_days",
                        "last_30_days",
                        "last_90_days",
                        "custom",
                    ],
                    "this_month",
                ),
            );
            for key in ["from", "to"] {
                spec.filters.insert(key.into(), Filter::string(""));
            }
        }
        let tables: &[(&str, &[(&str, &str)])] = if bookkeeping {
            &[
                (
                    "controlled-bookkeeping-changes",
                    &[
                        ("when", "When"),
                        ("set", "Set to"),
                        ("reason", "Reason"),
                        ("by", "By"),
                    ],
                ),
                (
                    "late-recordings-table",
                    &[
                        ("number", "#"),
                        ("date", "Transaction date"),
                        ("recorded", "Recorded"),
                        ("days", "Days"),
                        ("status", "Status"),
                        ("description", "Description"),
                        ("by", "Posted by"),
                        ("review", "Review"),
                    ],
                ),
            ]
        } else {
            &[
                (
                    "duty-separation-changes",
                    &[
                        ("when", "When"),
                        ("set", "Set to"),
                        ("reason", "Reason"),
                        ("by", "By"),
                    ],
                ),
                (
                    "exceptions-table",
                    &[
                        ("number", "#"),
                        ("when", "When"),
                        ("kind", "Kind"),
                        ("what", "What"),
                        ("prepared", "Prepared by"),
                        ("approved", "Approved by"),
                        ("reason", "Reason"),
                        ("review", "Review"),
                    ],
                ),
            ]
        };
        let columns = tables
            .iter()
            .map(|(id, fields)| {
                ListColumns::mount(
                    id,
                    fields
                        .iter()
                        .map(|(key, label)| BuiltinColumn::new(key, label))
                        .collect(),
                )
                .map_err(|_| Error::Internal)
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            pages,
            bookkeeping,
            list: ListState::parse(query, spec.shared().map_err(|_| Error::Internal)?)
                .map_err(|_| Error::Invalid)?,
            columns,
            browser_zone: None,
            period: None,
            dialog: None,
            text: String::new(),
            dates: HashMap::new(),
            flash: None,
        })
    }

    fn filter(&self, key: &str) -> &str {
        self.list.filters.get(key).map(String::as_str).unwrap_or("")
    }
    fn after(&self) -> Option<i64> {
        params::positive_id(Some(self.filter("after")))
    }
    fn reviewed(&self) -> Option<bool> {
        match self.filter("review") {
            "pending" => Some(false),
            "reviewed" => Some(true),
            _ => None,
        }
    }
    fn path(&self) -> &'static str {
        if self.bookkeeping {
            "/finance/reports/bookkeeping"
        } else {
            "/finance/reports/approval-exceptions"
        }
    }
    fn query(&self) -> HashMap<String, String> {
        let mut query = self.list.to_params();
        // ListState carries the cursor; only signed positive database numbers leave the page.
        query.remove("after");
        if let Some(after) = self.after() {
            query.insert("after".into(), after.to_string());
        }
        query
    }
    fn link(&self, key: &str, value: &str) -> String {
        let mut query = self.query();
        query.remove("after");
        if value.is_empty() || value == "all" {
            query.remove(key);
        } else {
            query.insert(key.into(), value.into());
        }
        params::address(self.path(), &query)
    }
    async fn allowed(&self, scope: &Scope, capability: &str) -> Result<bool> {
        let client = self.pages.reports.db.get().await?;
        match organisation::authorize_target(&**client, scope, company(scope)?, capability).await {
            Ok(_) => Ok(true),
            Err(Error::Forbidden | Error::Unauthorized | Error::NotFound) => Ok(false),
            Err(error) => Err(error),
        }
    }
    async fn display(&self, scope: &Scope) -> Result<DisplayContext> {
        let client = self.pages.reports.db.get().await?;
        let locale = self
            .pages
            .locales
            .resolve(&**client, scope, None)
            .await?
            .locale;
        match self
            .pages
            .display
            .display(&**client, scope, scope.actor().organisation(), locale)
            .await
        {
            // TODO(platform): Settings' company proof requires an unarchived
            // company even for display reads (also see Reports::layout).
            // Ledger history stays readable and a refused write must leave
            // its page alive, so fall back to the user's tenant clock.
            Err(Error::Forbidden) => {
                self.pages
                    .display
                    .display(&**client, scope, None, locale)
                    .await
            }
            result => result,
        }
    }
    fn sync_period(&mut self, display: &DisplayContext) -> Clock {
        let clock = Clock::resolve(display, self.browser_zone.as_deref(), Timestamp::now());
        let period = Period::parse(|key| self.list.filters.get(key).cloned(), clock.today);
        if let Some(filter) = &mut self.period {
            filter.set(period, &clock);
        } else {
            self.period = Some(PeriodFilter::new("exceptions-period", period, &clock));
        }
        clock
    }
    fn close(&mut self) {
        self.dialog = None;
        self.text.clear();
        self.dates.clear();
    }
    fn refused(&mut self, error: LedgerError) {
        let invalid = matches!(error, LedgerError::Invalid(_));
        let text = if invalid {
            match self.dialog {
                Some(Dialog::FiscalYear) => "Check the fiscal year's dates: it starts on the first of a month and runs to the end of one, up to two years.".into(),
                Some(Dialog::Review { .. }) if !self.bookkeeping => "Keep the note to 500 characters.".into(),
                _ => "Give a reason of 1 to 500 characters.".into(),
            }
        } else {
            refusal(&error, self.bookkeeping)
        };
        if !invalid {
            self.close();
        }
        self.flash = Some((StatusKind::Error, text));
    }
    fn success(&mut self, text: impl Into<String>) {
        self.close();
        self.flash = Some((StatusKind::Success, text.into()));
    }

    async fn event(&mut self, scope: &Scope, payload: &Value) -> Result<()> {
        if scope.actor().organisation().is_none() {
            self.close();
            self.flash = Some((
                StatusKind::Error,
                "Use an account affiliated with the company to open its ledger.".into(),
            ));
            return Ok(());
        }
        let event = payload["type"].as_str().unwrap_or("");
        let dialog_kind = payload["dialog"].as_str().or_else(|| {
            (payload.get("entry").is_some() || payload.get("exception").is_some())
                .then_some("review")
        });
        if let Some(index) = event
            .strip_prefix("control_grid_")
            .and_then(|s| s.parse::<usize>().ok())
        {
            if let Some(columns) = self.columns.get_mut(index) {
                columns.handle(&params::string_values(payload));
            }
            return Ok(());
        }
        if !self.bookkeeping {
            let display = self.display(scope).await?;
            let clock = self.sync_period(&display);
            if let Some(outcome) = self
                .period
                .as_mut()
                .and_then(|filter| filter.event(event, payload, &clock))
            {
                if let bilibi_datetime::filter::Outcome::Applied(period) = outcome {
                    self.list
                        .filters
                        .extend(period.params().into_iter().map(|(k, v)| (k.into(), v)));
                    self.list.filters.remove("after");
                }
                return Ok(());
            }
        }
        if event == "browser_timezone" {
            self.browser_zone = payload["timezone"]
                .as_str()
                .filter(|s| bilibi_datetime::valid_timezone(s))
                .map(str::to_owned);
            return Ok(());
        }
        if event == "clear-flash" {
            crate::web::clear_flash(&mut self.flash, payload);
            return Ok(());
        }
        if event == "close_dialog" {
            self.close();
            return Ok(());
        }
        if event == "text" {
            if let Some(text) = payload["dialog[text]"].as_str() {
                self.text = text.into();
            }
            for key in ["start_date", "end_date"] {
                if let Some(text) = payload[format!("dialog[{key}]")].as_str() {
                    self.dates.insert(key.into(), text.into());
                }
            }
            return Ok(());
        }
        let capability = match event {
            "turn_on" | "turn_off" if self.bookkeeping => MANAGE_CONTROLLED_BOOKKEEPING,
            "open_fiscal_year" if self.bookkeeping => MANAGE_PERIODS,
            "change_separation" if !self.bookkeeping => MANAGE_DUTY_SEPARATION,
            "review" => {
                if self.bookkeeping {
                    REVIEW_LATE_RECORDINGS
                } else {
                    REVIEW_EXCEPTIONS
                }
            }
            "open_dialog" => match dialog_kind {
                Some("controlled-off") if self.bookkeeping => MANAGE_CONTROLLED_BOOKKEEPING,
                Some("fiscal-year") if self.bookkeeping => MANAGE_PERIODS,
                Some("separation") if !self.bookkeeping => MANAGE_DUTY_SEPARATION,
                Some("review") => {
                    if self.bookkeeping {
                        REVIEW_LATE_RECORDINGS
                    } else {
                        REVIEW_EXCEPTIONS
                    }
                }
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        if !self.allowed(scope, capability).await? {
            self.refused(LedgerError::Unauthorized);
            return Ok(());
        }
        let ledger = self.pages.reports.ledger.clone();
        let company = company(scope)?;
        self.flash = None;
        if event == "open_dialog" {
            self.close();
            self.dialog = match dialog_kind {
                Some("controlled-off") => Some(Dialog::ControlledOff),
                Some("fiscal-year") => Some(Dialog::FiscalYear),
                Some("separation") => match ledger.get_duty_separation(scope, company).await {
                    Ok(state) => Some(Dialog::Separation {
                        required: !state.required,
                    }),
                    Err(error) => {
                        self.refused(error);
                        None
                    }
                },
                Some("review") => params::positive_id(
                    payload[if self.bookkeeping {
                        "entry"
                    } else {
                        "exception"
                    }]
                    .as_str(),
                )
                .map(|id| Dialog::Review { id }),
                _ => None,
            };
            return Ok(());
        }
        // Typed form values are kept intact for facade validation, including
        // 500-character Unicode reasons longer than generic form_values' 1 KiB.
        let text = match payload.get("dialog[text]") {
            Some(Value::String(text)) => text.clone(),
            None => self.text.clone(),
            _ => {
                self.refused(LedgerError::InvalidOption);
                return Ok(());
            }
        };
        self.text = text.clone();
        match (event, self.dialog) {
            ("turn_on", _) => match ledger
                .set_controlled_bookkeeping(scope, company, true, None)
                .await
            {
                Ok(_) => {
                    self.success("Bookkeeping is controlled: entries need an open fiscal period.")
                }
                Err(error) => self.refused(error),
            },
            ("turn_off", Some(Dialog::ControlledOff)) => match ledger
                .set_controlled_bookkeeping(scope, company, false, Some(&text))
                .await
            {
                Ok(_) => self
                    .success("Bookkeeping is not controlled; the change is listed as an override."),
                Err(error) => self.refused(error),
            },
            ("change_separation", Some(Dialog::Separation { required })) => match ledger
                .set_duty_separation(scope, company, required, Some(&text))
                .await
            {
                Ok(state) => self.success(if state.required {
                    "Separation of duties is required again."
                } else {
                    "Separation of duties is off; same-person approvals are listed here."
                }),
                Err(error) => self.refused(error),
            },
            ("open_fiscal_year", Some(Dialog::FiscalYear)) => {
                for key in ["start_date", "end_date"] {
                    match payload.get(format!("dialog[{key}]")) {
                        Some(Value::String(text)) => {
                            self.dates.insert(key.into(), text.into());
                        }
                        None => {}
                        Some(_) => {
                            self.flash = Some((StatusKind::Error, "Give the first day of the fiscal year as a date. The last day is optional, but must be a date.".into()));
                            return Ok(());
                        }
                    }
                }
                let date = |key: &str| -> std::result::Result<Option<Date>, ()> {
                    match self.dates.get(key).map(String::as_str).unwrap_or("") {
                        "" => Ok(None),
                        value => value.parse().map(Some).map_err(|_| ()),
                    }
                };
                let first = ledger
                    .get_controlled_bookkeeping(scope, company)
                    .await
                    .map(|s| s.fiscal_years == 0);
                // Bilimbi ignores the last day when the first is blank.
                let start = date("start_date");
                let end = match start {
                    Ok(None) => Ok(None),
                    _ => date("end_date"),
                };
                match (start, end, first) {
                    (_, _, Err(error)) => self.refused(error),
                    (Ok(start), Ok(end), Ok(first)) if start.is_some() || !first => {
                        match ledger.open_fiscal_year(scope, company, &NewFiscalYear { start_date: start, end_date: end }).await {
                            Ok(year) => self.success(format!("Fiscal year {} to {} is open.", year.start_date, year.end_date)), Err(error) => self.refused(error),
                        }
                    }
                    _ => self.flash = Some((StatusKind::Error, "Give the first day of the fiscal year as a date. The last day is optional, but must be a date.".into())),
                }
            }
            ("review", Some(Dialog::Review { id })) if self.bookkeeping => match ledger
                .record_late_recording_review(scope, company, id, &text)
                .await
            {
                Ok(review) => {
                    self.success(format!("Review of entry {} recorded.", review.entry_id))
                }
                Err(error) => self.refused(error),
            },
            ("review", Some(Dialog::Review { id })) => match ledger
                .review_approval_exception(scope, company, id, Some(&text))
                .await
            {
                Ok(exception) => self.success(format!(
                    "Review of exception {} recorded.",
                    exception.exception_number
                )),
                Err(error) => self.refused(error),
            },
            _ => self.refused(LedgerError::InvalidOption),
        }
        Ok(())
    }

    async fn render(&mut self, scope: &Scope) -> Result<String> {
        let title = if self.bookkeeping {
            "Bookkeeping"
        } else {
            "Approval exceptions"
        };
        let subtitle = if self.bookkeeping {
            "Controlled bookkeeping, fiscal years, and entries recorded late"
        } else {
            "Same-person approvals and overrides of controls, for someone independent to review."
        };
        let mut parts = vec![html(Header {
            title: Html::text(title),
            subtitle,
            ..Header::default()
        })?];
        if scope.actor().organisation().is_none() {
            self.close();
            parts.push(html(
                EmptyState::missing(
                    "No company is selected",
                    "Use an account affiliated with the company to open its ledger.",
                )
                .id("reports-company-missing"),
            )?);
            return PageView {
                id: if self.bookkeeping {
                    "bookkeeping-page"
                } else {
                    "approval-exceptions-page"
                },
                flash: self.flash_map(),
                parts,
            }
            .render()
            .map_err(|_| Error::Internal);
        }
        let display = self.display(scope).await?;
        if !self.allowed(scope, VIEW).await? {
            self.close();
            parts.push(self.read_refusal(&LedgerError::Unauthorized)?);
        } else {
            let result = if self.bookkeeping {
                self.bookkeeping_parts(scope, &display).await
            } else {
                self.exception_parts(scope, &display).await
            };
            match result {
                Ok(content) => parts.extend(content),
                Err(error) => {
                    self.close();
                    parts.push(self.read_refusal(&error)?);
                }
            }
            if self.dialog.is_some() {
                parts.push(self.dialog_view()?);
            }
        }
        PageView {
            id: if self.bookkeeping {
                "bookkeeping-page"
            } else {
                "approval-exceptions-page"
            },
            flash: self.flash_map(),
            parts,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    fn read_refusal(&self, error: &LedgerError) -> Result<Html> {
        if self.bookkeeping {
            html(Alert {
                id: "bookkeeping-refusal",
                kind: StatusKind::Error,
                class: "mb-3",
                body: Html::text(&refusal(error, true)),
            })
        } else if matches!(error, LedgerError::Unauthorized) {
            html(EmptyState::forbidden("view the ledger").id("exceptions-refusal"))
        } else {
            html(EmptyState::missing("The exceptions cannot be shown", "The company or the period could not be read. Choose another period or reload the page.").id("exceptions-refusal"))
        }
    }
    fn flash_map(&self) -> FlashMap<'_> {
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = text,
                StatusKind::Error => flash.error = text,
                StatusKind::Warning => flash.warning = text,
                StatusKind::Info => flash.info = text,
            }
        }
        flash
    }
    async fn bookkeeping_parts(
        &mut self,
        scope: &Scope,
        display: &DisplayContext,
    ) -> std::result::Result<Vec<Html>, LedgerError> {
        let ledger = self.pages.reports.ledger.clone();
        let company = company(scope)?;
        let controlled = ledger.get_controlled_bookkeeping(scope, company).await?;
        let mut changes = ledger
            .list_controlled_bookkeeping_changes(scope, company)
            .await?;
        changes.reverse();
        let entries = ledger
            .list_late_recordings(
                scope,
                company,
                &LateRecordingQuery {
                    status: match self.filter("status") {
                        "approaching" => Some(LateStatus::Approaching),
                        "late" => Some(LateStatus::Late),
                        _ => None,
                    },
                    reviewed: self.reviewed(),
                    limit: Some(100),
                    after: self.after(),
                },
            )
            .await?;
        let can_manage = self.allowed(scope, MANAGE_CONTROLLED_BOOKKEEPING).await?;
        let can_year = self.allowed(scope, MANAGE_PERIODS).await?;
        let can_review = self.allowed(scope, REVIEW_LATE_RECORDINGS).await?;
        let change = controlled.change.as_ref();
        let mut content = html(Summary {
            status_id: "controlled-bookkeeping-status",
            badge: html(Badge::status(
                if controlled.controlled {
                    BadgeKind::Success
                } else {
                    BadgeKind::Warning
                },
                if controlled.controlled {
                    "Controlled"
                } else {
                    "Not controlled"
                },
            ))?,
            since: change
                .map(|c| datetime(display, c.changed_at, "controlled-bookkeeping-changed-at"))
                .transpose()?
                .unwrap_or_default(),
            actor: change
                .map(|c| actor(Some(c.changed_by_user_id), c.changed_by_impersonator_id))
                .unwrap_or_default(),
            reason: change.and_then(|c| c.reason.as_deref()).unwrap_or(""),
            action: if can_manage {
                if controlled.controlled {
                    button(
                        "controlled-bookkeeping-off",
                        "open_dialog",
                        "dialog",
                        "controlled-off",
                        "Turn off",
                        ButtonVariant::Danger,
                    )?
                } else {
                    button(
                        "controlled-bookkeeping-on",
                        "turn_on",
                        "",
                        "",
                        "Turn on",
                        ButtonVariant::Secondary,
                    )?
                }
            } else {
                Html::EMPTY
            },
        })?;
        if controlled.controlled && controlled.fiscal_years == 0 {
            content.push(&html(Alert { id: "no-fiscal-year", kind: StatusKind::Warning, class: "mt-4", body: Html::text("No fiscal year is open, so no entry can be posted yet. Open the first fiscal year to start.") })?);
        }
        let years = if controlled.fiscal_years == 0 {
            "No fiscal year is open yet.".into()
        } else {
            format!(
                "{} fiscal {} open.",
                controlled.fiscal_years,
                if controlled.fiscal_years == 1 {
                    "year"
                } else {
                    "years"
                }
            )
        };
        if can_year {
            let year_action = button(
                "fiscal-year-open",
                "open_dialog",
                "dialog",
                "fiscal-year",
                if controlled.fiscal_years == 0 {
                    "Open the first fiscal year"
                } else {
                    "Open the next fiscal year"
                },
                if controlled.fiscal_years == 0 {
                    ButtonVariant::Primary
                } else {
                    ButtonVariant::Secondary
                },
            )?;
            content.push(&html(YearView {
                count: &years,
                action: year_action,
            })?);
        }
        if !changes.is_empty() {
            let rows = changes
                .iter()
                .map(|c| {
                    history_row(
                        "controlled-bookkeeping-change",
                        c.change_number,
                        c.controlled,
                        c.reason.as_deref(),
                        c.changed_by_user_id,
                        c.changed_by_impersonator_id,
                        c.changed_at,
                        display,
                        true,
                    )
                })
                .collect::<Result<_>>()?;
            content.push(&self.table(0, rows, "Changes to controlled bookkeeping", None)?);
        }
        let mut parts = vec![section("controlled-bookkeeping", "Controlled bookkeeping", "A controlled company posts an entry only into a fiscal period it has opened and left open, so nothing is recorded into a period nobody set up. Opening the first fiscal year is a step of its own; entries dated before it are refused, and so are dates after the last one until the next year is opened. Old transactions are not refused: an entry dated into an open period posts, however late.", content, false)?];
        let thresholds = bilibi_finance_ledger::Ledger::late_recording_thresholds();
        let description = format!("An entry is listed once it is recorded {} or more days after its transaction date, and is late past {} days. Each late entry needs a reason from an independent reviewer, someone who did not post it. The days are whole days from the transaction date to the day it was recorded.", thresholds.approaching_days, thresholds.late_days);
        let mut filters = self.tabs(
            "late-recordings-status",
            "Lateness",
            "status",
            &[
                ("all", "All"),
                ("approaching", "Approaching"),
                ("late", "Late"),
            ],
        )?;
        filters.push(&self.tabs(
            "late-recordings-review",
            "Review status",
            "review",
            &[
                ("all", "Any review"),
                ("pending", "Awaiting review"),
                ("reviewed", "Reviewed"),
            ],
        )?);
        parts.push(section(
            "late-recordings",
            "Entries recorded late",
            &description,
            html(FiltersView {
                id: "late-recordings-filters",
                class: "mt-3 flex flex-wrap items-center gap-4",
                content: filters,
            })?,
            false,
        )?);
        let compact = self.columns[1].mode() == RowMode::Compact;
        let mut rows = Vec::new();
        for entry in &entries {
            let mut row = row(
                &format!("late-entry-{}", entry.entry_number),
                &[
                    ("number", entry.entry_number.to_string()),
                    ("date", entry.entry_date.to_string()),
                    ("days", entry.days.to_string()),
                    (
                        "by",
                        actor(
                            Some(entry.posted_by_user_id),
                            entry.posted_by_impersonator_id,
                        ),
                    ),
                ],
            );
            row = row.slot(
                "recorded",
                datetime(
                    display,
                    entry.posted_at,
                    &format!("late-entry-at-{}", entry.entry_number),
                )?,
            );
            row = row.slot(
                "status",
                classification(
                    if entry.status == LateStatus::Late {
                        BadgeKind::Danger
                    } else {
                        BadgeKind::Warning
                    },
                    if entry.status == LateStatus::Late {
                        "Late"
                    } else {
                        "Approaching"
                    },
                )?,
            );
            row = row.slot(
                "description",
                html(CellView {
                    id: "",
                    main: &entry.description,
                    detail: entry.reference.as_deref().unwrap_or(""),
                    compact,
                })?,
            );
            let review = match &entry.review {
                Some(review) => html(CellView {
                    id: &format!("late-entry-review-{}", entry.entry_number),
                    main: &review.reason,
                    detail: &actor(
                        Some(review.reviewed_by_user_id),
                        review.reviewed_by_impersonator_id,
                    ),
                    compact,
                })?,
                None if can_review => review_button(
                    &format!("late-entry-review-open-{}", entry.entry_number),
                    "entry",
                    entry.entry_id,
                )?,
                None => Html::text("Awaiting review"),
            };
            rows.push(row.slot("review", review));
        }
        parts.push(section(
            "late-recordings-card",
            "",
            "",
            self.table(
                1,
                rows,
                "Entries recorded late",
                Some((
                    "late-recordings-empty",
                    "No entries recorded late",
                    "Nothing in the ledger was recorded this long after its transaction date.",
                )),
            )?,
            true,
        )?);
        if entries.len() == 100 {
            parts.push(self.next(
                "late-recordings-next",
                entries.last().expect("100 entries").entry_number,
                "Later entries",
            )?);
        }
        Ok(parts)
    }
    async fn exception_parts(
        &mut self,
        scope: &Scope,
        display: &DisplayContext,
    ) -> std::result::Result<Vec<Html>, LedgerError> {
        let ledger = self.pages.reports.ledger.clone();
        let company = company(scope)?;
        let clock = self.sync_period(display);
        let period = self.period.as_ref().expect("period initialized");
        let (from, before) = period
            .period
            .bounds(&clock.timezone)
            .map_err(|_| LedgerError::InvalidOption)?;
        let separation = ledger.get_duty_separation(scope, company).await?;
        let mut changes = ledger.list_duty_separation_changes(scope, company).await?;
        changes.reverse();
        let exceptions = ledger
            .list_approval_exceptions(
                scope,
                company,
                &ApprovalExceptionQuery {
                    recorded_from: Some(from),
                    recorded_before: Some(before),
                    reviewed: self.reviewed(),
                    limit: Some(100),
                    after: self.after(),
                },
            )
            .await?;
        let can_manage = self.allowed(scope, MANAGE_DUTY_SEPARATION).await?;
        let can_review = self.allowed(scope, REVIEW_EXCEPTIONS).await?;
        let change = separation.change.as_ref();
        let mut content = html(Summary {
            status_id: "duty-separation-status",
            badge: html(Badge::status(
                if separation.required {
                    BadgeKind::Success
                } else {
                    BadgeKind::Warning
                },
                if separation.required {
                    "Required"
                } else {
                    "Not required"
                },
            ))?,
            since: change
                .map(|c| datetime(display, c.changed_at, "duty-separation-changed-at"))
                .transpose()?
                .unwrap_or_default(),
            actor: change
                .map(|c| actor(Some(c.changed_by_user_id), c.changed_by_impersonator_id))
                .unwrap_or_default(),
            reason: change.and_then(|c| c.reason.as_deref()).unwrap_or(""),
            action: if can_manage {
                button(
                    "duty-separation-change",
                    "open_dialog",
                    "dialog",
                    "separation",
                    if separation.required {
                        "Turn off"
                    } else {
                        "Require again"
                    },
                    if separation.required {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Primary
                    },
                )?
            } else {
                Html::EMPTY
            },
        })?;
        if !changes.is_empty() {
            let rows = changes
                .iter()
                .map(|c| {
                    history_row(
                        "duty-separation-change",
                        c.change_number,
                        c.required,
                        c.reason.as_deref(),
                        c.changed_by_user_id,
                        c.changed_by_impersonator_id,
                        c.changed_at,
                        display,
                        false,
                    )
                })
                .collect::<Result<_>>()?;
            content.push(&self.table(0, rows, "Changes to separation of duties", None)?);
        }
        let mut parts = vec![section("duty-separation", "Separation of duties", "While it is required, nobody approves, posts, or finalises a record they prepared, nor one prepared by someone they impersonated. A company with one finance person turns it off: those approvals are then allowed and listed below.", content, false)?];
        let mut filters = html(self.period.as_ref().expect("period initialized"))?;
        filters.push(&self.tabs(
            "exceptions-review",
            "Review status",
            "review",
            &[
                ("all", "All"),
                ("pending", "Awaiting review"),
                ("reviewed", "Reviewed"),
            ],
        )?);
        parts.push(html(FiltersView {
            id: "exceptions-filters",
            class: "mt-4 mb-3 flex flex-wrap items-center gap-4",
            content: filters,
        })?);
        let compact = self.columns[1].mode() == RowMode::Compact;
        let mut rows = Vec::new();
        for exception in &exceptions {
            let mut row = row(
                &format!("exception-{}", exception.exception_number),
                &[
                    ("number", exception.exception_number.to_string()),
                    (
                        "prepared",
                        actor(
                            exception.prepared_by_user_id,
                            exception.prepared_by_impersonator_id,
                        ),
                    ),
                    (
                        "approved",
                        actor(
                            Some(exception.approved_by_user_id),
                            exception.approved_by_impersonator_id,
                        ),
                    ),
                    ("reason", exception.reason.clone()),
                ],
            );
            row = row.slot(
                "when",
                datetime(
                    display,
                    exception.recorded_at,
                    &format!("exception-at-{}", exception.exception_number),
                )?,
            );
            let mut kind = Html::default();
            if exception.same_person {
                kind.push(&classification(BadgeKind::Warning, "Same person")?);
            }
            if exception.overrides {
                kind.push(&classification(BadgeKind::Neutral, "Override")?);
            }
            row = row.slot("kind", kind);
            row = row.slot(
                "what",
                html(CellView {
                    id: "",
                    main: &format!(
                        "{}: {}",
                        source_label(&exception.source),
                        action_label(&exception.action)
                    ),
                    detail: &exception.subject,
                    compact,
                })?,
            );
            let review = match &exception.review {
                Some(review) => html(CellView {
                    id: &format!("exception-review-{}", exception.exception_number),
                    main: &actor(
                        Some(review.reviewed_by_user_id),
                        review.reviewed_by_impersonator_id,
                    ),
                    detail: review.note.as_deref().unwrap_or(""),
                    compact,
                })?,
                None if can_review => review_button(
                    &format!("exception-review-open-{}", exception.exception_number),
                    "exception",
                    exception.id,
                )?,
                None => Html::text("Awaiting review"),
            };
            rows.push(row.slot("review", review));
        }
        parts.push(section(
            "exceptions-card",
            "",
            "",
            self.table(
                1,
                rows,
                "Approval exceptions",
                Some((
                    "exceptions-empty",
                    "No approval exceptions",
                    "Nobody approved their own work or overrode a control in this period.",
                )),
            )?,
            true,
        )?);
        if exceptions.len() == 100 {
            parts.push(self.next(
                "exceptions-next",
                exceptions.last().expect("100 exceptions").exception_number,
                "Later exceptions",
            )?);
        }
        Ok(parts)
    }
    fn tabs(&self, id: &str, label: &str, key: &str, choices: &[(&str, &str)]) -> Result<Html> {
        let links: Vec<_> = choices
            .iter()
            .map(|(value, _)| self.link(key, value))
            .collect();
        html(Tabs {
            id,
            label,
            tabs: choices
                .iter()
                .zip(&links)
                .map(|((value, title), link)| Tab::patch(link, title, self.filter(key) == *value))
                .collect(),
            ..Tabs::default()
        })
    }
    fn next(&self, id: &str, after: i64, label: &str) -> Result<Html> {
        let mut query = self.query();
        query.insert("after".into(), after.to_string());
        html(NextLink {
            id,
            href: &params::address(self.path(), &query),
            label,
        })
    }
    fn table(
        &self,
        index: usize,
        rows: Vec<FlexRow>,
        caption: &str,
        empty: Option<(&str, &str, &str)>,
    ) -> Result<Html> {
        if rows.is_empty() {
            if let Some((id, title, reason)) = empty {
                return html(EmptyState::missing(title, reason).id(id));
            }
        }
        let columns = &self.columns[index];
        let event = format!("control_grid_{index}");
        let mut table = FlexTable::new(&columns.id, columns.flex_columns(), rows);
        table.zoom = columns.zoom;
        table.suggestions = columns.flex_suggestions();
        table.add_query = &columns.add_query;
        table.event = &event;
        table.caption = caption;
        table.framed = false;
        html(table)
    }
    fn dialog_view(&self) -> Result<Html> {
        let Some(dialog) = self.dialog else {
            return Ok(Html::default());
        };
        let (prefix, title, description, submit, label, working, variant, field_label, required) = match dialog {
            Dialog::ControlledOff => ("controlled-off", "Turn off controlled bookkeeping", "Entries may then post into a date no fiscal period covers, after the last year. The change is listed in the approval exceptions for an independent reviewer, with this reason.", "turn_off", "Turn off", "Saving…", ButtonVariant::Danger, "Reason (1 to 500 characters)", true),
            Dialog::FiscalYear => ("fiscal-year", "Open a fiscal year", "A fiscal year is divided into monthly periods, which are open until they are closed. The first year may start on any month; each later year starts the day after the previous one ends. Leave the end date blank for twelve months.", "open_fiscal_year", "Open fiscal year", "Opening…", ButtonVariant::Primary, "", false),
            Dialog::Separation { required: false } => ("duty-separation", "Turn separation of duties off", "Whoever prepares a record may then approve it. Each such approval is listed here with this reason, for someone independent to review.", "change_separation", "Turn off", "Saving…", ButtonVariant::Primary, "Reason (1 to 500 characters)", true),
            Dialog::Separation { required: true } => ("duty-separation", "Require separation of duties again", "Nobody may then approve, post, or finalise a record they prepared.", "change_separation", "Require again", "Saving…", ButtonVariant::Primary, "Reason (optional)", false),
            Dialog::Review { .. } if self.bookkeeping => ("late-review", "Record your review", "Your review is kept with your name and the time; it cannot be changed. You cannot review an entry you posted.", "review", "Record review", "Recording…", ButtonVariant::Primary, "Why it was recorded late (1 to 500 characters)", true),
            Dialog::Review { .. } => ("exception-review", "Record your review", "Your review is kept with your name and the time; it cannot be changed. You cannot review an approval you made or a record you prepared.", "review", "Record review", "Recording…", ButtonVariant::Primary, "Note (optional, up to 500 characters)", false),
        };
        let fields = if matches!(dialog, Dialog::FiscalYear) {
            vec![
                Field::in_form("dialog", "start_date")
                    .id("fiscal-year-start")
                    .label("First day of the year")
                    .kind(InputKind::Date)
                    .value(
                        self.dates
                            .get("start_date")
                            .map(String::as_str)
                            .unwrap_or(""),
                    )
                    .required(),
                Field::in_form("dialog", "end_date")
                    .id("fiscal-year-end")
                    .label("Last day (optional)")
                    .kind(InputKind::Date)
                    .value(self.dates.get("end_date").map(String::as_str).unwrap_or("")),
            ]
        } else {
            let id = if prefix == "exception-review" {
                "exception-review-note".into()
            } else if prefix == "duty-separation" {
                "duty-separation-reason".into()
            } else {
                format!("{prefix}-reason")
            };
            let field = Field::in_form("dialog", "text")
                .id(id)
                .label(field_label)
                .kind(InputKind::Textarea)
                .value(&self.text)
                .attr("maxlength", "500");
            vec![if required { field.required() } else { field }]
        };
        let form = html(FormView {
            id: &format!("{prefix}-form"),
            fields,
            submit,
            cancel_id: &format!("{prefix}-close"),
            confirm_id: &format!("{prefix}-confirm"),
            label,
            working,
            variant,
        })?;
        html(Modal {
            id: &format!("{prefix}-modal"),
            title,
            description,
            cancel_event: "close_dialog",
            body: form,
            flash: Some(self.flash_map()),
            ..Modal::default()
        })
    }
}

fn company(scope: &Scope) -> Result<i64> {
    scope
        .actor()
        .organisation()
        .map(|id| id.get())
        .ok_or(Error::Forbidden)
}
fn html(fragment: impl Template) -> Result<Html> {
    Html::render(&fragment).map_err(|_| Error::Internal)
}
fn actor(user: Option<i64>, impersonator: Option<i64>) -> String {
    match (user, impersonator) {
        (None, _) => "—".into(),
        (Some(user), None) => format!("User #{user}"),
        (Some(user), Some(by)) => format!("User #{user} (impersonated by #{by})"),
    }
}
fn button(
    id: &str,
    event: &str,
    name: &str,
    value: &str,
    label: &str,
    variant: ButtonVariant,
) -> Result<Html> {
    html(Button {
        id,
        event,
        name,
        value,
        variant,
        button_type: "button",
        body: Html::text(label),
        ..Button::default()
    })
}
fn review_button(id: &str, name: &str, value: i64) -> Result<Html> {
    button(
        id,
        "open_dialog",
        name,
        &value.to_string(),
        "Record review",
        ButtonVariant::Secondary,
    )
}
fn row(id: &str, cells: &[(&str, String)]) -> FlexRow {
    let mut row = FlexRow::new(id);
    row.dom_id = Some(id.into());
    for (key, value) in cells {
        row = row.slot(key, Html::text(value));
    }
    row
}
#[allow(clippy::too_many_arguments)]
fn history_row(
    prefix: &str,
    number: i64,
    enabled: bool,
    reason: Option<&str>,
    user: i64,
    by: Option<i64>,
    at: Timestamp,
    display: &DisplayContext,
    bookkeeping: bool,
) -> Result<FlexRow> {
    let label = match (bookkeeping, enabled) {
        (true, true) => "Controlled",
        (true, false) => "Not controlled",
        (false, true) => "Required",
        (false, false) => "Not required",
    };
    Ok(row(
        &format!("{prefix}-{number}"),
        &[
            ("set", label.into()),
            ("reason", reason.unwrap_or("—").into()),
            ("by", actor(Some(user), by)),
        ],
    )
    .slot(
        "when",
        datetime(display, at, &format!("{prefix}-at-{number}"))?,
    ))
}
fn section(id: &str, title: &str, description: &str, content: Html, flat: bool) -> Result<Html> {
    let heading = format!("{id}-heading");
    let mut body = if title.is_empty() {
        Html::default()
    } else {
        html(SectionHeading {
            id: &heading,
            title,
            description,
            ..SectionHeading::default()
        })?
    };
    body.push(&content);
    html(Card {
        id,
        inner_class: match id {
            "late-recordings" => "p-5 sm:p-6 mt-4",
            "late-recordings-card" => "p-0 mt-3",
            _ if flat => "p-0",
            _ => "p-5 sm:p-6",
        },
        role: "region",
        labelledby: if title.is_empty() { "" } else { &heading },
        body,
        ..Card::default()
    })
}
fn datetime(display: &DisplayContext, at: Timestamp, id: &str) -> Result<Html> {
    let rendered = display.render(at, Format::DateTime, Precision::Second);
    html(TimeView {
        id,
        instant: bilibi_ui::flex_table::Instant {
            precision: "second",
            ..bilibi_ui::flex_table::Instant::new(
                &rendered.iso,
                &rendered.text,
                &rendered.company,
                &rendered.utc,
                rendered.mode.as_str(),
            )
        },
    })
}
fn source_label(source: &str) -> &str {
    match source {
        "finance/ledger" => "Ledger",
        "finance/payables" => "Payables",
        "finance/reconciliation" => "Bank reconciliation",
        _ => source,
    }
}
fn action_label(action: &str) -> String {
    match action {
        "document_approval" => "Document approved".into(),
        "match_override" => "Match exceptions approved".into(),
        "payment_approval" => "Payment approved".into(),
        "journal_approval" => "Journal approved".into(),
        "reconciliation_finalisation" => "Reconciliation finalised".into(),
        "period_reopen" => "Period reopened".into(),
        "fiscal_year_reopen" => "Fiscal year reopened".into(),
        _ => {
            let text = action.replace('_', " ");
            let mut chars = text.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        }
    }
}
fn refusal(error: &LedgerError, bookkeeping: bool) -> String {
    match error {
        LedgerError::Unauthorized => "You do not have permission for that action.",
        LedgerError::CompanyArchived => "The company is archived; nothing can be changed.",
        LedgerError::BookNotOpen => "The ledger's book is not open yet.",
        LedgerError::EntryNotFound => "That entry no longer exists.",
        LedgerError::ExceptionNotFound => "That exception no longer exists.",
        LedgerError::AlreadyReviewed if bookkeeping => "Someone has already reviewed that entry.",
        LedgerError::AlreadyReviewed => "Someone has already reviewed that exception.",
        LedgerError::NotLateRecorded => "That entry was not recorded late.",
        LedgerError::ReviewerNotIndependent if bookkeeping => {
            "An entry is reviewed by someone who did not post it."
        }
        LedgerError::ReviewerNotIndependent => {
            "An exception is reviewed by someone who neither approved nor prepared what it records."
        }
        _ => return format!("That could not be done ({error})."),
    }
    .into()
}
#[derive(Template)]
#[template(path = "control_page.html")]
struct PageView<'a> {
    id: &'a str,
    flash: FlashMap<'a>,
    parts: Vec<Html>,
}
#[derive(Template)]
#[template(path = "control_summary.html")]
struct Summary<'a> {
    status_id: &'a str,
    badge: Html,
    since: Html,
    actor: String,
    reason: &'a str,
    action: Html,
}
#[derive(Template)]
#[template(path = "control_cell.html")]
struct CellView<'a> {
    id: &'a str,
    main: &'a str,
    detail: &'a str,
    compact: bool,
}
#[derive(Template)]
#[template(path = "control_form.html")]
struct FormView<'a> {
    id: &'a str,
    submit: &'a str,
    fields: Vec<Field>,
    cancel_id: &'a str,
    confirm_id: &'a str,
    label: &'a str,
    working: &'a str,
    variant: ButtonVariant,
}
#[derive(Template)]
#[template(
    source = "{% import \"flex_table.html\" as table %}{% call table::instant(id=id, i=instant) %}{% endcall %}",
    ext = "html"
)]
struct TimeView<'a> {
    id: &'a str,
    instant: bilibi_ui::flex_table::Instant,
}

impl Page<Scope, Error> for Pages {
    fn title(&self, operation: &str) -> Option<&'static str> {
        match operation {
            "bookkeeping" => Some("Bookkeeping"),
            "approval-exceptions" => Some("Approval exceptions"),
            _ => None,
        }
    }
    fn render<'a>(&'a self, request: PageRequest<'a, Scope>) -> PageFuture<'a, Error> {
        Box::pin(async move {
            let mut state = State::new(self.clone(), request.operation, request.query)?;
            Ok(PageBody {
                html: state
                    .render(request.scope.ok_or(Error::Unauthorized)?)
                    .await?,
                live: true,
                title: None,
            })
        })
    }
    fn mount<'a>(&'a self, request: PageRequest<'a, Scope>) -> MountFuture<'a, Scope, Error> {
        Box::pin(async move {
            let state: Box<dyn LiveState<Scope, Error>> =
                Box::new(State::new(self.clone(), request.operation, request.query)?);
            Ok(Some(state))
        })
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match request.input {
                LiveInput::Event(payload) => self.event(request.scope, payload).await?,
                LiveInput::Params(query) => {
                    self.list = ListState::parse(
                        query,
                        self.list
                            .spec()
                            .clone()
                            .shared()
                            .map_err(|_| Error::Internal)?,
                    )
                    .map_err(|_| Error::Invalid)?;
                    self.period = None;
                    self.close();
                }
                LiveInput::Refresh => {}
            }
            Ok(LiveBody {
                html: self.render(request.scope).await?,
                query: self.query(),
                ..LiveBody::default()
            })
        })
    }
}

#[derive(Template)]
#[template(path = "control_filters.html")]
struct FiltersView<'a> {
    id: &'a str,
    class: &'a str,
    content: Html,
}
/// Bilimbi's plain "Later …" patch link under a capped list.
#[derive(Template)]
#[template(
    source = r#"<div class="mt-3"><a id="{{ id }}" href="{{ href }}" data-patch class="text-action hover:underline">{{ label }}</a></div>"#,
    ext = "html"
)]
struct NextLink<'a> {
    id: &'a str,
    href: &'a str,
    label: &'a str,
}
#[derive(Template)]
#[template(path = "control_year.html")]
struct YearView<'a> {
    count: &'a str,
    action: Html,
}

fn classification(kind: BadgeKind, text: &str) -> Result<Html> {
    html(Badge {
        dot: false,
        ..Badge::status(kind, text)
    })
}
