//! A form for the existing report layout setting; no JSON editing required.
use super::*;
use bilibi_agent_api::page::Call;
use bilibi_ui::{Button, ButtonVariant, Card, Choice, Field, Form, Header, InputKind, PageVariant};
use serde_json::json;

pub(super) struct State {
    pages: Pages,
    sections: Vec<Value>,
    keys: Vec<usize>,
    next_key: usize,
    accounts: Vec<Choice>,
    flash: Option<(StatusKind, String)>,
    errors: HashMap<String, Vec<String>>,
}
impl State {
    pub(super) async fn new(pages: Pages, scope: &Scope) -> Result<Self> {
        let input = json!({"company_id":company(scope)?});
        let value = pages
            .contracts
            .read(scope, "finance.reports.sections.get", &input)
            .await
            .map_err(|_| Error::Internal)?;
        let sections = value["result"]["sections"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut accounts = vec![Choice::new("Choose an account", "")];
        let mut input = input;
        loop {
            let page = pages
                .contracts
                .read(scope, "finance.ledger.account.list", &input)
                .await
                .map_err(|_| Error::Internal)?;
            for account in page["result"]["records"].as_array().into_iter().flatten() {
                if let (Some(code), Some(name)) =
                    (account["code"].as_str(), account["name"].as_str())
                {
                    // A section may list a group: it gathers everything under it.
                    let group = if account["is_group"].as_bool() == Some(true) {
                        " (group)"
                    } else {
                        ""
                    };
                    accounts.push(Choice::new(format!("{code} {name}{group}"), code));
                }
            }
            match page["result"]["next_after_code"].as_str() {
                Some(code) => input["after_code"] = json!(code),
                None => break,
            }
        }
        let keys = (0..sections.len()).collect();
        let next_key = sections.len();
        Ok(Self {
            pages,
            sections,
            keys,
            next_key,
            accounts,
            flash: None,
            errors: HashMap::new(),
        })
    }
    fn capture(&mut self, payload: &Value) {
        let values: HashMap<_, _> = payload
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(key, value)| {
                Some((
                    key.strip_prefix("layout[")?.strip_suffix(']')?.to_owned(),
                    value.as_str()?.to_owned(),
                ))
            })
            .collect();
        for (index, section) in self.sections.iter_mut().enumerate() {
            for key in ["report", "name"] {
                if let Some(value) = values.get(&format!("{key}_{index}")) {
                    section[key] = json!(value);
                }
            }
            if let Some(accounts) = section["accounts"].as_array_mut() {
                for (number, account) in accounts.iter_mut().enumerate() {
                    if let Some(value) = values.get(&format!("account_{index}_{number}")) {
                        *account = json!(value);
                    }
                }
            }
        }
    }
    fn render(&self) -> Result<String> {
        let mut content = render(Header {
            title: Html::text("Report sections"),
            title_actions: super::menu_pin(
                "report-sections-pin",
                "nav-finance-reports-sections",
                "Report sections",
            )?,
            subtitle: "Choose headings and accounts for your reports.",
            ..Header::default()
        })?;
        content.push(&render(Button {
            href: PATH,
            body: Html::text("Reports"),
            ..Button::default()
        })?);
        let mut fields = Html::EMPTY;
        for (index, section) in self.sections.iter().enumerate() {
            let key = self.keys[index];
            let mut body = render(
                Field::in_form("layout", &format!("report_{index}"))
                    .id(format!("report-section-{key}-report"))
                    .label("Report")
                    .kind(InputKind::Select)
                    .value(section["report"].as_str().unwrap_or("profit_and_loss"))
                    .options(vec![
                        Choice::new("Profit and loss", "profit_and_loss"),
                        Choice::new("Balance sheet", "balance_sheet"),
                        Choice::new("Cash position", "cash_position"),
                    ]),
            )?;
            body.push(&render(
                Field::in_form("layout", &format!("name_{index}"))
                    .id(format!("report-section-{key}-name"))
                    .label("Heading")
                    .required()
                    .errors(
                        self.errors
                            .get(&format!("sections.{index}.name"))
                            .cloned()
                            .unwrap_or_default(),
                    )
                    .value(section["name"].as_str().unwrap_or_default()),
            )?);
            for (number, account) in section["accounts"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
            {
                let code = account.as_str().unwrap_or_default();
                let mut options = self.accounts.clone();
                if !code.is_empty() && !options.iter().any(|o| o.value == code) {
                    options.push(Choice::new(format!("{code} (not in the chart)"), code));
                }
                body.push(&render(
                    Field::in_form("layout", &format!("account_{index}_{number}"))
                        .id(format!("report-section-{key}-account-{number}"))
                        .label("Account")
                        .kind(InputKind::Select)
                        .value(code)
                        .options(options),
                )?);
            }
            body.push(&render(Button {
                id: &format!("report-section-{key}-add-account"),
                event: "section_account",
                name: "index",
                value: &index.to_string(),
                button_type: "button",
                body: Html::text("Add account"),
                ..Button::default()
            })?);
            body.push(&render(Button {
                id: &format!("report-section-{key}-remove"),
                event: "section_remove",
                name: "index",
                value: &index.to_string(),
                button_type: "button",
                body: Html::text("Remove section"),
                ..Button::default()
            })?);
            fields.push(&render(Card {
                id: &format!("report-section-{key}"),
                body,
                ..Card::default()
            })?);
        }
        let mut actions = render(Button {
            id: "report-section-add",
            event: "section_add",
            button_type: "button",
            body: Html::text("Add section"),
            ..Button::default()
        })?;
        actions.push(&render(Button {
            id: "report-sections-save",
            event: "save_sections",
            button_type: "button",
            variant: ButtonVariant::Primary,
            body: Html::text("Save sections"),
            ..Button::default()
        })?);
        content.push(&render(Form {
            id: "report-sections-form",
            event: "layout_change",
            change: true,
            body: fields,
            actions,
            ..Form::default()
        })?);
        let mut flash = NO_FLASH;
        if let Some((kind, text)) = &self.flash {
            match kind {
                StatusKind::Success => flash.success = text,
                _ => flash.error = text,
            }
        }
        #[derive(Template)]
        #[template(
            source = "{% import \"components.html\" as ui %}{% import \"feedback.html\" as feedback %}{% call feedback::flash_group(messages=flash) %}{% endcall %}{% call ui::page(id=\"report-sections\", variant=variant) %}{{ body }}{% endcall %}",
            ext = "html"
        )]
        struct View<'a> {
            flash: FlashMap<'a>,
            variant: PageVariant,
            body: Html,
        }
        View {
            flash,
            variant: PageVariant::Form,
            body: content,
        }
        .render()
        .map_err(|_| Error::Internal)
    }
    pub(super) fn body(&self) -> Result<String> {
        self.render()
    }
}
impl LiveState<Scope, Error> for State {
    fn update<'a>(&'a mut self, request: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            if let LiveInput::Event(value) = request.input {
                let event = value["type"].as_str().unwrap_or_default();
                match event {
                    "layout_change" => {
                        self.capture(value);
                        self.errors.clear();
                    }
                    "section_add" => {
                        if self.sections.len() < 128 {
                            self.sections.push(
                                json!({"report":"profit_and_loss","name":"","accounts":[""]}),
                            );
                            self.keys.push(self.next_key);
                            self.next_key += 1;
                        }
                    }
                    "section_remove" | "section_account" => {
                        if let Some(index) = value["index"]
                            .as_str()
                            .and_then(|s| s.parse::<usize>().ok())
                            .filter(|i| *i < self.sections.len())
                        {
                            if event == "section_remove" {
                                self.sections.remove(index);
                                self.keys.remove(index);
                            } else {
                                self.sections[index]["accounts"]
                                    .as_array_mut()
                                    .ok_or(Error::Internal)?
                                    .push(json!(""));
                            }
                        }
                    }
                    "save_sections" => {
                        let mut sections = self.sections.clone();
                        for section in &mut sections {
                            if let Some(accounts) = section["accounts"].as_array_mut() {
                                accounts.retain(|a| a.as_str().is_some_and(|s| !s.is_empty()));
                            }
                        }
                        let input =
                            json!({"company_id":company(request.scope)?,"sections":sections});
                        let call = Call {
                            scope: request.scope,
                            credential: request.credential,
                            service: "finance/reports.section-pages",
                            event: "save_sections",
                            contract: "finance.reports.sections.set",
                        };
                        self.flash = Some(match self.pages.contracts.submit(&call, input).await {
                            Ok(_) => {
                                self.sections = sections;
                                (StatusKind::Success, "Report sections saved.".into())
                            }
                            Err(bilibi_agent_api::Error::InvalidInput(fields)) => {
                                let text = fields
                                    .values()
                                    .flatten()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                self.errors = fields.into_iter().collect();
                                (StatusKind::Error, text)
                            }
                            Err(bilibi_agent_api::Error::Forbidden) => (
                                StatusKind::Error,
                                "You do not have permission to arrange reports.".into(),
                            ),
                            Err(_) => (
                                StatusKind::Error,
                                "Sections were not saved. Reload and try again.".into(),
                            ),
                        });
                    }
                    "clear-flash" => clear_flash(&mut self.flash, value),
                    _ => {}
                }
            }
            Ok(LiveBody {
                html: self.render()?,
                ..LiveBody::default()
            })
        })
    }
}
