//! Bilimbi Sst.Web.ReturnsLive: the recent taxable periods of the company's
//! registration with each registered tax's return for the period (open,
//! finalised with its version, or none yet, with a control that prepares
//! it), a form that prepares the return of any period, and every return on
//! record. Preparing is a facade call that re-asks its capability.
use super::{
    clear_flash, columns, company, flash, format as f, render, table, Pages, DENIED, INDEX, SETUP,
};
use crate::{Registration, SstError, TaxReturn, TaxType, TaxablePeriod, PREPARE_RETURNS};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, Button, Cell, Choice, EmptyState, Field, FlashMap, FlexRow, Html, InputKind,
    ListColumns, RecordLink, StatusKind,
};
use jiff::civil::Date;
use std::collections::HashMap;

pub(super) struct Returns {
    pages: Pages,
    registration: Option<Registration>,
    periods: Vec<TaxablePeriod>,
    returns: Option<Vec<TaxReturn>>,
    refusal: Option<SstError>,
    can_prepare: bool,
    form: HashMap<String, String>,
    flash: Option<(StatusKind, String)>,
    framed: bool,
    period_columns: ListColumns,
    list_columns: ListColumns,
}

impl Returns {
    pub(super) fn new(pages: Pages) -> Result<Self> {
        Ok(Self {
            pages,
            registration: None,
            periods: vec![],
            returns: None,
            refusal: None,
            can_prepare: false,
            form: HashMap::from([
                ("tax_type".into(), "sales_tax".into()),
                ("period_start".into(), String::new()),
            ]),
            flash: None,
            framed: false,
            period_columns: columns(
                "sst-periods-table",
                &[("period", "Period"), ("due", "Due")],
                &[],
            )?,
            list_columns: columns(
                "sst-returns-table",
                &[
                    ("number", "No."),
                    ("tax", "Tax"),
                    ("period", "Period"),
                    ("status", "Status"),
                    ("due", "Due"),
                    ("tax_payable", "Tax payable"),
                ],
                &["tax_payable"],
            )?,
        })
    }

    pub(super) fn framed(mut self, framed: bool) -> Self {
        self.framed = framed;
        self
    }

    fn tax_types(&self) -> Vec<TaxType> {
        self.registration.as_ref().map_or_else(Vec::new, |r| {
            TaxType::ALL
                .into_iter()
                .filter(|t| r.number(*t).is_some())
                .collect()
        })
    }

    pub(super) async fn reload(&mut self, scope: &Scope) -> Result<()> {
        let sst = &self.pages.sst;
        let loaded = async {
            let company = company(scope).ok_or(SstError::Unauthorized)?;
            let registration = sst.get_registration(scope, company).await?;
            let periods = sst.taxable_periods(scope, company, None, Some(6)).await?;
            let returns = sst
                .list_returns(scope, company, None, Some(500), None)
                .await?;
            Ok::<_, SstError>((registration, periods, returns))
        }
        .await;
        match loaded {
            Ok((registration, periods, mut returns)) => {
                returns.reverse();
                self.registration = registration;
                self.periods = periods;
                self.returns = Some(returns);
                self.refusal = None;
                self.can_prepare = self.pages.allowed(scope, PREPARE_RETURNS).await?;
            }
            Err(e) => {
                self.registration = None;
                self.periods.clear();
                self.returns = None;
                self.refusal = Some(e);
                self.can_prepare = false;
            }
        }
        // One column per registered tax, as Bilimbi's `:for` column.
        let mut cols = vec![("period", "Period"), ("due", "Due")];
        let taxes = self.tax_types();
        cols.extend(taxes.iter().map(|t| (t.as_str(), f::tax_name(*t))));
        if !self
            .period_columns
            .builtins()
            .iter()
            .map(|c| c.id.as_str())
            .eq(cols.iter().map(|(id, _)| *id))
        {
            self.period_columns = columns("sst-periods-table", &cols, &[])?;
        }
        self.period_columns
            .load(&self.periods, |p| p.period_start.to_string());
        if let Some(returns) = &self.returns {
            self.list_columns.load(returns, |r| r.id.to_string());
        }
        Ok(())
    }

    /// The return that stands for each tax and period: an open one, else the
    /// latest finalised version; discarded returns stay in the list below.
    fn standing(&self, tax_type: TaxType, period_start: Date) -> Option<&TaxReturn> {
        let group: Vec<&TaxReturn> = self
            .returns
            .iter()
            .flatten()
            .filter(|r| {
                r.tax_type == tax_type
                    && r.period_start == period_start
                    && r.status != crate::ReturnStatus::Discarded
            })
            .collect();
        group
            .iter()
            .find(|r| r.status == crate::ReturnStatus::Open)
            .or_else(|| group.iter().max_by_key(|r| r.version))
            .copied()
    }

    pub(super) fn body(&self) -> Result<String> {
        let actions = render(Link {
            link_id: "sst-returns-setup",
            href: SETUP.into(),
            class: "",
            text: "SST setup".into(),
        })?;
        let refusal = self
            .refusal
            .as_ref()
            .map(|e| {
                render(
                    EmptyState::missing("SST returns unavailable", f::refusal(e))
                        .id("sst-returns-refusal"),
                )
            })
            .transpose()?;
        let unregistered = (self.refusal.is_none() && self.registration.is_none())
            .then(|| {
                render(
                    EmptyState::missing(
                        "Not registered yet",
                        "Record the company's SST registration in SST setup to prepare returns.",
                    )
                    .id("sst-returns-unregistered"),
                )
            })
            .transpose()?;
        let periods = match &self.registration {
            Some(registration) => Some(self.periods_card(registration)?),
            None => None,
        };
        let list = match &self.returns {
            Some(returns) => Some(self.returns_table(returns)?),
            None => None,
        };
        View {
            flash: flash(&self.flash),
            actions,
            refusal,
            unregistered,
            periods,
            list,
            count: self.returns.as_ref().map(Vec::len),
        }
        .render()
        .map_err(|_| Error::Internal)
    }

    fn periods_card(&self, registration: &Registration) -> Result<PeriodsView> {
        let taxes = self.tax_types();
        let mut rows = Vec::new();
        for p in &self.periods {
            let mut row = FlexRow::new(p.period_start.to_string());
            row.dom_id = Some(format!("period-{}", p.period_start));
            row = row
                .cell(
                    "period",
                    Cell::text(format!("{} to {}", p.period_start, p.period_end)),
                )
                .cell("due", Cell::text(p.due_date.to_string()));
            for tax in &taxes {
                let html = match self.standing(*tax, p.period_start) {
                    None if self.can_prepare => render(Button {
                        id: &format!("prepare-{tax}-{}", p.period_start),
                        event: "prepare",
                        name: "period",
                        value: &format!("{tax}|{}", p.period_start),
                        button_type: "button",
                        body: Html::text("Prepare"),
                        ..Button::default()
                    })?,
                    None => render(Muted { text: "none" })?,
                    Some(r) => {
                        let mut html = render(Link {
                            link_id: &format!("period-{tax}-{}-return", p.period_start),
                            href: format!("{INDEX}/returns/{}", r.id),
                            class: "",
                            text: format!("Return {}", r.return_number),
                        })?;
                        html.push(&f::badge(
                            &f::status_label(r),
                            f::status_kind(r.status),
                            "ml-1",
                        )?);
                        html
                    }
                };
                row = row.slot(tax.as_str(), html);
            }
            rows.push(row);
        }
        let table = table(
            &self.period_columns,
            "periods_grid",
            "Taxable periods",
            rows,
            None,
            false,
        )?;
        let forbidden = (!self.can_prepare)
            .then(|| {
                let mut empty =
                    EmptyState::forbidden("prepare SST returns").id("sst-prepare-forbidden");
                empty.class = "p-5".into();
                render(empty)
            })
            .transpose()?;
        let value = |k: &str| self.form.get(k).cloned().unwrap_or_default();
        Ok(PeriodsView {
            description: format!("{}-month periods; a return is due by the date shown (moved to the next working day when it falls on a weekend or holiday).", registration.period_months),
            table,
            forbidden,
            can_prepare: self.can_prepare,
            tax_field: Field::in_form("prepare", "tax_type")
                .id("prepare-tax-type")
                .kind(InputKind::Select)
                .label("Tax")
                .options(
                    taxes
                        .iter()
                        .map(|t| Choice::new(f::tax_name(*t), t.as_str()))
                        .collect(),
                )
                .value(value("tax_type")),
            period_field: Field::in_form("prepare", "period_start")
                .id("prepare-period-start")
                .kind(InputKind::Date)
                .label("Period starts")
                .value(value("period_start"))
                .required(),
        })
    }

    fn returns_table(&self, returns: &[TaxReturn]) -> Result<Html> {
        let mut rows = Vec::new();
        for r in returns {
            let mut row = FlexRow::new(r.id.to_string());
            row.dom_id = Some(format!("return-{}", r.return_number));
            let mut status = f::badge(&f::status_label(r), f::status_kind(r.status), "")?;
            if r.amends_return_id.is_some() {
                status.push(&render(Note { text: "amendment" })?);
            }
            rows.push(
                row.slot(
                    "number",
                    render(RecordLink {
                        workspace: self.framed,
                        kind: crate::MODULE,
                        record_id: &r.id.to_string(),
                        href: &format!("{INDEX}/returns/{}", r.id),
                        class: "font-medium hover:underline",
                        body: Html::text(&r.return_number.to_string()),
                    })?,
                )
                .cell("tax", Cell::text(f::tax_name(r.tax_type)))
                .cell(
                    "period",
                    Cell::text(format!("{} to {}", r.period_start, r.period_end)),
                )
                .slot("status", status)
                .cell("due", Cell::text(r.due_date.to_string()))
                .cell(
                    "tax_payable",
                    Cell::text(f::money(r.tax_payable, r.minor_units.unwrap_or(0))),
                ),
            );
        }
        table(
            &self.list_columns,
            "returns_grid",
            "Returns on record",
            rows,
            Some((
                "No returns yet",
                "Prepare the first return from its taxable period.",
            )),
            false,
        )
    }

    async fn prepare(
        &mut self,
        scope: &Scope,
        tax_type: &str,
        period_start: &str,
    ) -> Result<LiveBody> {
        if !self.pages.allowed(scope, PREPARE_RETURNS).await? {
            self.flash = Some((StatusKind::Error, DENIED.into()));
            return self.live(false, scope).await;
        }
        let opened = async {
            let company = company(scope).ok_or(SstError::Unauthorized)?;
            let tax_type: TaxType = tax_type.parse()?;
            let period_start: Date = period_start
                .trim()
                .parse()
                .map_err(|_| SstError::NotATaxablePeriod)?;
            self.pages
                .sst
                .open_return(scope, company, tax_type, period_start)
                .await
        }
        .await;
        match opened {
            Ok(r) => Ok(LiveBody {
                redirect: Some(format!("{INDEX}/returns/{}", r.id)),
                flash: Some((
                    StatusKind::Success,
                    format!("Return {} opened.", r.return_number),
                )),
                ..LiveBody::default()
            }),
            Err(e) => {
                self.flash = Some((StatusKind::Error, f::refusal(&e)));
                self.live(true, scope).await
            }
        }
    }

    async fn live(&mut self, reload: bool, scope: &Scope) -> Result<LiveBody> {
        if reload {
            self.reload(scope).await?;
        }
        Ok(LiveBody {
            html: self.body()?,
            ..LiveBody::default()
        })
    }
}

impl LiveState<Scope, Error> for Returns {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let v = match r.input {
                LiveInput::Refresh | LiveInput::Params(_) => return self.live(true, r.scope).await,
                LiveInput::Event(v) => v,
            };
            match v["type"].as_str() {
                Some("clear-flash") => {
                    clear_flash(&mut self.flash, v);
                    self.live(false, r.scope).await
                }
                Some("periods_grid") => {
                    self.period_columns.handle(&params::string_values(v));
                    self.live(false, r.scope).await
                }
                Some("returns_grid") => {
                    self.list_columns.handle(&params::string_values(v));
                    self.live(false, r.scope).await
                }
                Some("prepare_change") => {
                    self.form.extend(params::form_values(v, "prepare"));
                    self.live(false, r.scope).await
                }
                Some("prepare") => {
                    // A period's button sends `tax|start`; the form sends its fields.
                    let (tax_type, period_start) = match v["period"].as_str() {
                        Some(period) => {
                            let (t, s) = period.split_once('|').unwrap_or((period, ""));
                            (t.to_owned(), s.to_owned())
                        }
                        None => {
                            self.form.extend(params::form_values(v, "prepare"));
                            let get = |k: &str| self.form.get(k).cloned().unwrap_or_default();
                            (get("tax_type"), get("period_start"))
                        }
                    };
                    self.prepare(r.scope, &tax_type, &period_start).await
                }
                _ => Err(Error::Invalid),
            }
        })
    }
}

#[derive(Template)]
#[template(path = "returns.html")]
struct View<'a> {
    flash: FlashMap<'a>,
    actions: Html,
    refusal: Option<Html>,
    unregistered: Option<Html>,
    periods: Option<PeriodsView>,
    list: Option<Html>,
    count: Option<usize>,
}

struct PeriodsView {
    description: String,
    table: Html,
    forbidden: Option<Html>,
    can_prepare: bool,
    tax_field: Field,
    period_field: Field,
}

#[derive(Template)]
#[template(
    source = r#"<a id="{{ link_id }}" href="{{ href }}"{% if !class.is_empty() %} class="{{ class }}"{% endif %}>{{ text }}</a>"#,
    ext = "html"
)]
pub(super) struct Link<'a> {
    pub link_id: &'a str,
    pub href: String,
    pub class: &'a str,
    pub text: String,
}

#[derive(Template)]
#[template(
    source = r#"<span class="text-ink-muted">{{ text }}</span>"#,
    ext = "html"
)]
struct Muted<'a> {
    text: &'a str,
}

#[derive(Template)]
#[template(
    source = r#"<span class="ml-1 text-xs text-ink-muted">{{ text }}</span>"#,
    ext = "html"
)]
struct Note<'a> {
    text: &'a str,
}
