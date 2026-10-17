//! Bilimbi Payables.Web.BillsLive: filters, paging and built-in ListColumns.
use super::{company, format, render, Pages, INDEX};
use crate::{Document, DocumentPageQuery};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, BadgeKind, BuiltinColumn, Cell, Choice, EmptyState, Filter, FilterToolbar, FlexRow,
    FlexTable, Html, ListColumns, ListSpec, ListState, PageCounts, Pagination, RecordLink,
    TableEmpty, ToolbarControl,
};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

const SIZES: [u64; 3] = [25, 50, 100];
static SPEC: LazyLock<Arc<ListSpec>> = LazyLock::new(|| {
    ListSpec {
        page_sizes: SIZES.to_vec(),
        filters: [
            (
                "status".into(),
                Filter::one_of(&["", "draft", "approved", "held", "posted", "voided"], ""),
            ),
            (
                "document_type".into(),
                Filter::one_of(&["", "bill", "debit_note"], ""),
            ),
        ]
        .into(),
        ..ListSpec::default()
    }
    .shared()
    .expect("bills list spec")
});

pub(super) struct Bills {
    pages: Pages,
    list: ListState,
    columns: ListColumns,
    documents: Vec<Document>,
    total: u64,
    unavailable: bool,
    framed: bool,
}

impl Bills {
    pub(super) fn new(pages: Pages, query: &HashMap<String, String>, framed: bool) -> Result<Self> {
        let columns = [
            ("number", "Number"),
            ("document_type", "Type"),
            ("party", "Party"),
            ("party_document_number", "Party's number"),
            ("document_date", "Date"),
            ("total", "Total"),
            ("status", "Status"),
            ("match_status", "Match"),
        ]
        .into_iter()
        .map(|(id, label)| {
            let c = BuiltinColumn::new(id, label);
            if id == "total" {
                c.right()
            } else {
                c
            }
        })
        .collect();
        Ok(Self {
            pages,
            list: ListState::parse(query, SPEC.clone()).map_err(|_| Error::Internal)?,
            columns: ListColumns::mount("payables-bills-table", columns)
                .map_err(|_| Error::Internal)?,
            documents: vec![],
            total: 0,
            unavailable: false,
            framed,
        })
    }

    pub(super) async fn reload(&mut self, scope: &Scope) -> Result<()> {
        let result = match company(scope) {
            Some(company) => {
                self.pages
                    .payables
                    .page_documents(
                        scope,
                        company,
                        &DocumentPageQuery {
                            document_type: params::blank_to_none(
                                self.list.filters.get("document_type").map(String::as_str),
                            )
                            .map(str::to_owned),
                            status: params::blank_to_none(
                                self.list.filters.get("status").map(String::as_str),
                            )
                            .map(str::to_owned),
                            search: self.list.search.clone(),
                            page: self.list.page as i64,
                            page_size: self.list.page_size as i64,
                        },
                    )
                    .await
            }
            None => Err(crate::PayablesError::Unauthorized),
        };
        self.unavailable = result.is_err();
        self.documents.clear();
        self.total = 0;
        if let Ok(page) = result {
            self.documents = page.documents;
            self.total = page.total as u64;
            self.list.page = page.page as u64;
        }
        self.columns.load(&self.documents, |d| d.id.to_string());
        Ok(())
    }

    pub(super) fn body(&self) -> Result<String> {
        let mut held = self.list.clone();
        held.apply_filters(&HashMap::from([("status".into(), "held".into())]));
        let actions = render(HeldLink {
            href: params::address(INDEX, &held.to_params()),
        })?;
        let mut content = Html::EMPTY;
        let mut toolbar = FilterToolbar::new(
            "payables-bills-filters",
            vec![
                ToolbarControl::search(
                    "payables-bills-search",
                    "search",
                    "Search bills",
                    &self.list.search,
                    "Search by party or party's number…",
                ),
                ToolbarControl::select(
                    "payables-bills-status",
                    "status",
                    "Filter by status",
                    &self.list.filters["status"],
                    [
                        ("All statuses", ""),
                        ("draft", "draft"),
                        ("approved", "approved"),
                        ("held", "held"),
                        ("posted", "posted"),
                        ("voided", "voided"),
                    ]
                    .map(|(l, v)| Choice::new(l, v))
                    .to_vec(),
                ),
                ToolbarControl::select(
                    "payables-bills-type",
                    "document_type",
                    "Filter by type",
                    &self.list.filters["document_type"],
                    [
                        ("All types", ""),
                        ("Bill", "bill"),
                        ("Debit note", "debit_note"),
                    ]
                    .map(|(l, v)| Choice::new(l, v))
                    .to_vec(),
                ),
            ],
        );
        toolbar.event = "filter";
        content.push(&render(toolbar)?);
        if self.unavailable {
            let mut empty = EmptyState::missing(
                "Bills unavailable",
                "This company's bills cannot be read with your current access.",
            );
            empty.id = "payables-bills-unavailable".into();
            content.push(&render(empty)?);
        } else {
            let mut rows = Vec::new();
            for d in &self.documents {
                let mut row = FlexRow::new(d.id.to_string());
                row.dom_id = Some(format!("payables-bill-{}", d.id));
                row = row
                    .slot(
                        "number",
                        render(RecordLink {
                            workspace: self.framed,
                            kind: crate::MODULE,
                            record_id: &d.id.to_string(),
                            href: &format!("{INDEX}/{}", d.id),
                            class: "font-medium text-ink-strong hover:underline",
                            body: Html::text(&format::number(d.document_number)),
                        })?,
                    )
                    .slot(
                        "document_type",
                        format::badge(format::kind(&d.document_type), BadgeKind::Neutral, false)?,
                    )
                    .cell("party", Cell::text(&d.party.name))
                    .cell(
                        "party_document_number",
                        Cell::text(format::optional(&d.party_document_number)),
                    )
                    .cell("document_date", Cell::text(d.document_date.to_string()))
                    .slot(
                        "total",
                        render(Tabular {
                            text: format!("{} {}", d.total, d.currency_code),
                        })?,
                    )
                    .slot("status", format::status(&d.status)?)
                    .slot(
                        "match_status",
                        match &d.match_status {
                            Some(s) => format::match_badge(s, "")?,
                            None => Html::text("—"),
                        },
                    );
                rows.push(row);
            }
            let mut table =
                FlexTable::new("payables-bills-table", self.columns.flex_columns(), rows);
            table.event = "bills_grid";
            table.caption = "Bills";
            table.framed = false;
            table.zoom = self.columns.zoom;
            table.suggestions = self.columns.flex_suggestions();
            table.add_query = &self.columns.add_query;
            table.empty = Some(TableEmpty::Content(Html::text("No documents match the current filters. Bills are recorded through the payables API.")));
            content.push(&render(table)?);
            let mut pager = Pagination::new(
                "payables-bills-pagination",
                PageCounts::new(self.list.page, self.list.page_size, self.total),
                &SIZES,
            );
            pager.filters_event = "filter";
            content.push(&render(pager)?);
        }
        View { actions, content }
            .render()
            .map_err(|_| Error::Internal)
    }
}

#[derive(Template)]
#[template(path = "bills.html")]
struct View {
    actions: Html,
    content: Html,
}

// Base UI's Button link has no patch attribute. This is Bilimbi's plain
// patch link; the platform transport owns in-place Params navigation.
#[derive(Template)]
#[template(
    source = r#"<a id="payables-bills-held" href="{{ href }}" data-patch>Held bills</a>"#,
    ext = "html"
)]
struct HeldLink {
    href: String,
}

impl LiveState<Scope, Error> for Bills {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            let reload = match r.input {
                LiveInput::Refresh => true,
                LiveInput::Params(values) => {
                    self.list =
                        ListState::parse(values, SPEC.clone()).map_err(|_| Error::Internal)?;
                    true
                }
                LiveInput::Event(v) => match v["type"].as_str() {
                    Some("filter") => {
                        self.list.apply_filters(&params::form_values(v, "filters"));
                        true
                    }
                    Some("page") => {
                        self.list.put_page(v["page"].as_str());
                        true
                    }
                    Some("bills_grid") => {
                        self.columns.handle(&params::string_values(v));
                        false
                    }
                    _ => return Err(Error::Invalid),
                },
            };
            if reload {
                self.reload(r.scope).await?;
            }
            Ok(LiveBody {
                html: self.body()?,
                query: self.list.to_params(),
                ..LiveBody::default()
            })
        })
    }
}

#[derive(askama::Template)]
#[template(
    source = r#"<span class="tabular-nums">{{ text }}</span>"#,
    ext = "html"
)]
struct Tabular {
    text: String,
}
