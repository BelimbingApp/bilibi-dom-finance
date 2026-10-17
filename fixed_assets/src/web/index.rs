//! Bilimbi FixedAssets.Web.ListPage and its three small page adapters.
use super::{
    input::{self, Draft},
    *,
};
use crate::{RegisterReport, ReportPins, ACQUIRE, CONFIGURE, DEPRECIATE, REVERSE};
use bilibi_datetime::{filter::Outcome, Clock, Period, PeriodFilter};
use bilibi_ui::{
    Choice, FilterToolbar, FlashMap, InputKind, ListSpec, ListState, Pagination, RecordLink,
    ToolbarControl,
};
use std::sync::LazyLock;

static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        page_sizes: vec![25, 50, 100],
        default_page_size: 25,
        page_size_param: "page_size".into(),
        ..ListSpec::default()
    }
    .shared()
    .expect("fixed assets list spec")
});
const PINS: [&str; 4] = [
    "through_entry_number",
    "through_asset_number",
    "through_event_number",
    "through_category_revision_id",
];
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Register,
    Categories,
    Runs,
}
impl Kind {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "register" => Ok(Self::Register),
            "categories" => Ok(Self::Categories),
            "runs" => Ok(Self::Runs),
            _ => Err(Error::NotFound),
        }
    }
    fn key(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::Categories => "categories",
            Self::Runs => "runs",
        }
    }
    fn title(self) -> &'static str {
        match self {
            Self::Register => "Fixed assets",
            Self::Categories => "Asset categories",
            Self::Runs => "Depreciation runs",
        }
    }
    fn heading(self) -> &'static str {
        match self {
            Self::Register => "Register acquisition",
            Self::Categories => "Record category defaults",
            Self::Runs => "Run monthly depreciation",
        }
    }
    fn cap(self) -> &'static str {
        match self {
            Self::Register => ACQUIRE,
            Self::Categories => CONFIGURE,
            Self::Runs => DEPRECIATE,
        }
    }
    fn columns(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Register => &[
                ("number", "Number"),
                ("description", "Asset"),
                ("cost", "Cost"),
                ("accumulated", "Depreciation"),
                ("carrying", "Carrying value"),
                ("status", "Status"),
            ],
            Self::Categories => &[
                ("id", "Category ID"),
                ("name", "Category"),
                ("method", "Method"),
                ("life", "Life (months)"),
                ("revision", "Revision"),
            ],
            Self::Runs => &[
                ("id", "Run ID"),
                ("period", "Period"),
                ("total", "Depreciation"),
                ("entry", "Entry"),
                ("reversed", "Reversed"),
                ("exceptions", "Skipped assets"),
            ],
        }
    }
}
pub(super) struct Index {
    pages: Pages,
    kind: Kind,
    list: ListState,
    columns: ListColumns,
    framed: bool,
    period: Option<PeriodFilter>,
    query: HashMap<String, String>,
    browser_zone: Option<String>,
    draft: Draft,
    reverse: Draft,
    flash: Option<(StatusKind, String)>,
}
#[derive(Template)]
#[template(path = "index.html")]
struct View<'a> {
    page_dom: String,
    title: &'static str,
    flash: FlashMap<'a>,
    toolbar: Html,
    period: Html,
    unavailable: Html,
    table: Html,
    pager: Html,
    reconciliation: Html,
    heading: &'static str,
    form: Html,
    reverse: Html,
}
#[derive(Template)]
#[template(path = "reconciliation.html")]
struct ReconciliationView<'a> {
    report: &'a RegisterReport,
    reproduce: String,
    rows: Vec<String>,
}
impl Index {
    pub(super) fn new(
        pages: &Pages,
        operation: &str,
        query: &HashMap<String, String>,
        framed: bool,
    ) -> Result<Self> {
        let kind = Kind::parse(operation)?;
        Ok(Self {
            pages: pages.clone(),
            kind,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: cols(&format!("assets-{}-table", kind.key()), kind.columns())?,
            framed,
            period: None,
            query: query.clone(),
            browser_zone: None,
            draft: Draft::default(),
            reverse: Draft::default(),
            flash: None,
        })
    }
    fn pins(&self) -> ReportPins {
        let pin = |key| {
            self.query
                .get(key)
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|n| *n >= 0)
        };
        ReportPins {
            through_entry_number: pin(PINS[0]),
            through_asset_number: pin(PINS[1]),
            through_event_number: pin(PINS[2]),
            through_category_revision_id: pin(PINS[3]),
        }
    }
    async fn clock(&self, s: &Scope) -> Result<Clock> {
        Ok(Clock::resolve(
            &self.pages.display(s).await?,
            self.browser_zone.as_deref(),
            jiff::Timestamp::now(),
        ))
    }
    async fn period(&mut self, s: &Scope) -> Result<()> {
        let clock = self.clock(s).await?;
        let parsed = Period::parse(|k| self.query.get(k).cloned(), clock.today);
        if let Some(filter) = &mut self.period {
            filter.set(parsed, &clock);
        } else {
            self.period = Some(
                PeriodFilter::new("assets-report-period", parsed, &clock).label("Report through"),
            );
        }
        Ok(())
    }
    fn canonical(&self) -> HashMap<String, String> {
        let mut query = self.list.to_params();
        // The reference preserves these boundaries on filter/page changes too.
        for key in PINS {
            if let Some(v) = self.query.get(key) {
                if v.parse::<i64>().is_ok_and(|n| n >= 0) {
                    query.insert(key.into(), v.clone());
                }
            }
        }
        if let Some(p) = &self.period {
            query.extend(p.period.params().map(|(k, v)| (k.into(), v)));
        }
        query
    }
    fn fields(&self) -> Vec<Field> {
        use InputKind::{Date, Number, Select, Text};
        let d = &self.draft;
        let mut fields = Vec::new();
        let specs: &[(&str, &str, InputKind, bool)] = match self.kind {
            Kind::Categories => &[
                (
                    "category_id",
                    "Existing category ID (blank creates one)",
                    Number,
                    false,
                ),
                ("name", "New category name", Text, false),
                ("cost_account_id", "Cost account ID", Number, true),
                (
                    "accumulated_account_id",
                    "Accumulated depreciation account ID",
                    Number,
                    true,
                ),
                (
                    "expense_account_id",
                    "Depreciation expense account ID",
                    Number,
                    true,
                ),
                ("gain_account_id", "Disposal gain account ID", Number, true),
                ("loss_account_id", "Disposal loss account ID", Number, true),
            ],
            Kind::Register => &[
                ("source", "Acquisition source", Select, false),
                ("category_revision_id", "Category revision ID", Number, true),
                ("description", "Asset description", Text, true),
                ("entry_id", "Manual entry ID", Number, false),
                (
                    "entry_line_number",
                    "Manual entry line number",
                    Number,
                    false,
                ),
                ("bill_id", "Supplier bill ID", Number, false),
                (
                    "bill_line_number",
                    "Supplier bill line number",
                    Number,
                    false,
                ),
                ("in_service_date", "In-service date", Date, true),
                (
                    "residual_value",
                    "Residual value (blank means zero)",
                    Text,
                    false,
                ),
            ],
            Kind::Runs => &[("period", "First day of month", Date, true)],
        };
        for (key, label, kind, required) in specs {
            let mut f = d.field(key, label, *kind, *required, "asset");
            if *key == "source" {
                f = f.options(vec![
                    Choice::new("Manual ledger entry", "entry"),
                    Choice::new("Posted supplier bill", "bill"),
                ]);
                if d.value(key).is_empty() {
                    f = f.value("entry");
                }
            }
            fields.push(f);
        }
        if self.kind == Kind::Categories {
            fields.push(d.method_field());
            fields.extend([
                d.field(
                    "life_months",
                    "Default useful life (months)",
                    Number,
                    true,
                    "asset",
                ),
                d.field(
                    "annual_rate",
                    "Annual percentage (reducing balance)",
                    Text,
                    false,
                    "asset",
                ),
                d.field("reason", "Reason", Text, true, "asset"),
            ]);
        }
        fields
    }
    async fn write_form(&self, s: &Scope) -> Result<Html> {
        if !self.pages.allowed(s, self.kind.cap()).await? {
            return withheld(
                "assets-write-withheld",
                "Read-only access",
                "Ask your company administrator for permission to record this operation.",
            );
        }
        render(Operation {
            form_dom: "assets-write-form",
            event_name: "record_asset",
            button_dom: "assets-record",
            verb: "Record",
            fields: self.fields(),
            help: if self.kind == Kind::Runs {
                "One charge through this month, including any scheduled depreciation still unposted. Closed periods are refused."
            } else {
                ""
            },
        })
    }
    async fn reverse_form(&self, s: &Scope) -> Result<Html> {
        if self.kind != Kind::Runs {
            return Ok(Html::EMPTY);
        }
        let form = if self.pages.allowed(s, REVERSE).await? {
            render(Operation {
                form_dom: "assets-reverse-form",
                event_name: "reverse",
                button_dom: "assets-reverse",
                verb: "Post reversal",
                help: "",
                fields: vec![
                    self.reverse
                        .field("run_id", "Run ID", InputKind::Number, true, "run-reversal"),
                    self.reverse.field(
                        "effective_date",
                        "Reversal date in an open period",
                        InputKind::Date,
                        true,
                        "run-reversal",
                    ),
                    self.reverse
                        .field("reason", "Reason", InputKind::Text, true, "run-reversal"),
                ],
            })?
        } else {
            withheld(
                "assets-reverse-withheld",
                "Reversals unavailable",
                "Ask your company administrator for asset reversal permission.",
            )?
        };
        render(bilibi_ui::Card {
            title: "Reverse a run",
            class: "mt-4",
            body: form,
            ..Default::default()
        })
    }
    async fn rows(
        &self,
        s: &Scope,
    ) -> crate::Result<(Vec<(String, FlexRow)>, Option<RegisterReport>)> {
        let c = company(s).ok_or(FixedAssetsError::CompanyNotFound)?;
        let mut rows = Vec::new();
        let report = match self.kind {
            Kind::Register => {
                let as_of = self.period.as_ref().expect("period initialized").period.to;
                let report = self
                    .pages
                    .assets
                    .register_report(s, c, as_of, &self.pins())
                    .await?;
                for r in &report.rows {
                    let a = &r.asset;
                    let link = render(RecordLink {
                        workspace: self.framed,
                        kind: MODULE,
                        record_id: &a.id.to_string(),
                        href: &format!("/finance/fixed-assets/register/{}", a.id),
                        body: Html::text(&a.description),
                        class: "",
                    })?;
                    let row = FlexRow::new(a.id.to_string())
                        .cell("number", Cell::text(a.asset_number.to_string()))
                        .slot("description", link)
                        .cell("cost", Cell::text(a.cost.to_string()))
                        .cell("accumulated", Cell::text(r.accumulated.to_string()))
                        .cell("carrying", Cell::text(r.carrying_value.to_string()))
                        .cell(
                            "status",
                            Cell::text(if r.voided {
                                "Voided"
                            } else if r.disposed {
                                "Disposed"
                            } else {
                                "In service"
                            }),
                        );
                    rows.push((format!("{} {}", a.asset_number, a.description), row));
                }
                Some(report)
            }
            Kind::Categories => {
                for c in self.pages.assets.list_categories(s, c).await? {
                    let row = FlexRow::new(c.id.to_string())
                        .cell("id", Cell::text(c.id.to_string()))
                        .cell("name", Cell::text(c.name.clone()))
                        .cell(
                            "method",
                            Cell::text(input::method(c.revision.defaults.assumptions.method)),
                        )
                        .cell(
                            "life",
                            Cell::text(c.revision.defaults.assumptions.life_months.to_string()),
                        )
                        .cell("revision", Cell::text(c.revision.id.to_string()));
                    rows.push((c.name, row));
                }
                None
            }
            Kind::Runs => {
                for r in self.pages.assets.list_runs(s, c).await? {
                    let skipped = if r.exceptions.is_empty() {
                        "None".into()
                    } else {
                        r.exceptions
                            .iter()
                            .map(|e| format!("#{} {}", e.asset_number, e.description))
                            .collect::<Vec<_>>()
                            .join(", ")
                    };
                    let row = FlexRow::new(r.id.to_string())
                        .cell("id", Cell::text(r.id.to_string()))
                        .cell("period", Cell::text(r.period.to_string()))
                        .cell("total", Cell::text(r.total.to_string()))
                        .cell(
                            "entry",
                            Cell::text(
                                r.entry_number
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "No charge".into()),
                            ),
                        )
                        .cell(
                            "reversed",
                            Cell::text(if r.reversed { "Yes" } else { "No" }),
                        )
                        .cell("exceptions", Cell::text(skipped));
                    rows.push((r.period.to_string(), row));
                }
                None
            }
        };
        for (_, row) in &mut rows {
            row.dom_id = Some(format!("asset-row-{}", row.key));
        }
        Ok((rows, report))
    }
    async fn body(&mut self, s: &Scope) -> Result<String> {
        let (mut rows, report, unavailable) = match self.rows(s).await {
            Ok((rows, r)) => (rows, r, Html::EMPTY),
            Err(e) => (
                vec![],
                None,
                render(
                    EmptyState::missing("Records unavailable", refusal(e)).id("assets-unavailable"),
                )?,
            ),
        };
        let search = self.list.search.to_lowercase();
        rows.retain(|(text, _)| text.to_lowercase().contains(&search));
        let page = self.list.paginate(&rows);
        let counts = page.counts;
        let rows = page.entries.iter().map(|(_, row)| row.clone()).collect();
        let table = if unavailable.is_empty() {
            table(
                &format!("assets-{}-table", self.kind.key()),
                &self.columns,
                rows,
                self.kind.title(),
                "grid",
                "No records match these filters.",
            )?
        } else {
            Html::EMPTY
        };
        let mut toolbar = FilterToolbar::new(
            "assets-filters",
            vec![ToolbarControl::search(
                "assets-search",
                "search",
                "Search",
                &self.list.search,
                "Search records…",
            )],
        );
        toolbar.event = "filter";
        let mut pager = Pagination::new("assets-pagination", counts, &[25, 50, 100]);
        pager.filters_event = "filter";
        let reconciliation = if let Some(r) = &report {
            let mut query = self.canonical();
            for (key, value) in PINS.into_iter().zip([
                r.through_entry_number,
                r.through_asset_number,
                r.through_event_number,
                r.through_category_revision_id,
            ]) {
                query.insert(key.into(), value.to_string());
            }
            render(ReconciliationView {
                report: r,
                reproduce: params::address("/finance/fixed-assets/register", &query),
                rows: r
                    .reconciliation
                    .iter()
                    .map(|r| {
                        format!(
                            "Account {}: ledger {}, register {}, difference {}.",
                            r.account_id, r.ledger_balance, r.register_balance, r.difference
                        )
                    })
                    .collect(),
            })?
        } else {
            Html::EMPTY
        };
        let view = View {
            page_dom: format!("assets-{}", self.kind.key()),
            title: self.kind.title(),
            flash: flash_map(&self.flash),
            toolbar: render(toolbar)?,
            period: if self.kind == Kind::Register {
                render(self.period.as_ref().expect("period initialized").clone())?
            } else {
                Html::EMPTY
            },
            unavailable,
            table,
            pager: render(pager)?,
            reconciliation,
            heading: self.kind.heading(),
            form: self.write_form(s).await?,
            reverse: self.reverse_form(s).await?,
        };
        view.render().map_err(|_| Error::Internal)
    }
    async fn record(&self, s: &Scope) -> crate::Result<Option<crate::Run>> {
        let c = company(s).ok_or(FixedAssetsError::CompanyNotFound)?;
        let d = &self.draft;
        match self.kind {
            Kind::Categories => {
                let input = d.revision()?;
                if let Some(id) = d.optional_id("category_id")? {
                    self.pages.assets.revise_category(s, c, id, &input).await?;
                } else {
                    self.pages
                        .assets
                        .create_category(s, c, d.value("name"), &input)
                        .await?;
                }
            }
            Kind::Register => {
                let units = self.pages.ledger.get_book(s, c).await?.minor_units;
                let input = d.acquisition(units)?;
                let (id, line) = if d.value("source") == "bill" {
                    (d.id("bill_id")?, d.id("bill_line_number")?)
                } else {
                    (d.id("entry_id")?, d.id("entry_line_number")?)
                };
                let line = i32::try_from(line).map_err(|_| {
                    crate::invalid(
                        if d.value("source") == "bill" {
                            "bill_line_number"
                        } else {
                            "entry_line_number"
                        },
                        "is invalid",
                    )
                })?;
                if d.value("source") == "bill" {
                    self.pages
                        .assets
                        .acquire_from_bill(s, c, id, line, &input)
                        .await?;
                } else {
                    self.pages
                        .assets
                        .acquire_from_entry(s, c, id, line, &input)
                        .await?;
                }
            }
            Kind::Runs => {
                return self
                    .pages
                    .assets
                    .depreciate(
                        s,
                        c,
                        d.date("period")
                            .map_err(|_| FixedAssetsError::InvalidPeriod)?,
                    )
                    .await
                    .map(Some)
            }
        }
        Ok(None)
    }
    async fn event(&mut self, s: &Scope, v: &serde_json::Value) -> Result<()> {
        let event = v["event"].as_str().unwrap_or("");
        let clock = self.clock(s).await?;
        if let Some(outcome) = self.period.as_mut().and_then(|p| p.event(event, v, &clock)) {
            if let Outcome::Applied(p) = outcome {
                self.query.extend(p.params().map(|(k, v)| (k.into(), v)));
                self.list.put_page(Some("1"));
            }
            return Ok(());
        }
        match event {
            "filter" => self.list.apply_filters(&params::form_values(v, "filters")),
            "page" => self.list.put_page(v["page"].as_str()),
            "grid" => {
                self.columns.handle(&params::string_values(v));
            }
            "browser_timezone" => {
                self.browser_zone = v["timezone"].as_str().map(str::to_owned);
                self.period(s).await?;
            }
            "record_asset" => {
                self.draft = Draft::posted(v);
                if !self.pages.allowed(s, self.kind.cap()).await? {
                    self.flash = Some((StatusKind::Error, DENIED.into()));
                    return Ok(());
                }
                match self.record(s).await {
                    Ok(run) => {
                        self.flash = Some(
                            if let Some(r) = run.filter(|r| !r.exceptions.is_empty()) {
                                (StatusKind::Error,format!("Recorded without asset {}: its acquisition entry was reversed. Void the acquisition from the asset page.",r.exceptions.iter().map(|e|format!("#{}",e.asset_number)).collect::<Vec<_>>().join(", ")))
                            } else {
                                (StatusKind::Success, "Recorded.".into())
                            },
                        );
                        self.draft = Draft::default();
                    }
                    Err(e) => {
                        self.draft.refused(&e);
                        self.flash = Some((StatusKind::Error, refusal(e)));
                    }
                }
            }
            "reverse" => {
                if self.kind != Kind::Runs {
                    return Err(Error::Invalid);
                }
                self.reverse = Draft::posted(v);
                if !self.pages.allowed(s, REVERSE).await? {
                    self.flash = Some((StatusKind::Error, DENIED.into()));
                    return Ok(());
                }
                let result = async {
                    self.pages
                        .assets
                        .reverse_run(
                            s,
                            company(s).ok_or(FixedAssetsError::CompanyNotFound)?,
                            self.reverse.id("run_id")?,
                            &self.reverse.event()?,
                        )
                        .await
                }
                .await;
                match result {
                    Ok(_) => {
                        self.flash = Some((StatusKind::Success, "Depreciation reversed.".into()))
                    }
                    Err(e) => {
                        self.reverse.refused(&e);
                        self.flash = Some((StatusKind::Error, refusal(e)));
                    }
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(())
    }
}
impl LiveState<Scope, Error> for Index {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            self.flash = None;
            self.period(r.scope).await?;
            match r.input {
                LiveInput::Refresh => {}
                LiveInput::Params(query) => {
                    self.query = query.clone();
                    self.list =
                        ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?;
                    self.period(r.scope).await?;
                }
                LiveInput::Event(v) => self.event(r.scope, v).await?,
            }
            let html = self.body(r.scope).await?;
            Ok(LiveBody {
                html,
                query: self.canonical(),
                ..Default::default()
            })
        })
    }
}
