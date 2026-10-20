//! Recording a bill or a debit note, and correcting a draft. Bilimbi recorded
//! documents through its API; this page saves the same draft through the
//! `finance.payables.bill.create` and `.debit_note.create` contracts the agent
//! doors call, and leaves approving and posting to the bill's own page.
use super::{company, format as f, record_id, render, Pages, Written, INDEX};
use crate::{Document, DocumentPageQuery, Party, TaxCode, APPROVE_DOCUMENTS, CREATE_DOCUMENTS};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_finance_ledger::{Account, AccountType, MinorUnits, Money};
use bilibi_tenancy::Scope;
use bilibi_ui::{
    error_sentence,
    page::{LiveBody, LiveFuture, LiveInput, LiveRequest, LiveState},
    params, AddMore, Alert, BackLink, Button, ButtonVariant, Choice, Combobox, EmptyState,
    ErrorSummary, Fact, Facts, Field, Html, InputKind, StatusKind,
};
use rust_decimal::{Decimal, RoundingStrategy};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

const MAX_LINES: usize = 100;
// The browser sends a field as `bill[<key>]`; a line's are `line_<id>_<field>`.
const PREFIX: &str = "bill";

#[derive(Clone, Default)]
struct Line {
    id: u32,
    description: String,
    account: String,
    tax: String,
    amount: String,
}

#[derive(Default)]
struct Draft {
    pick: String,
    key: String,
    name: String,
    tax_id: String,
    registration: String,
    address: String,
    number: String,
    date: String,
    due: String,
    description: String,
    lines: Vec<Line>,
    next: u32,
}

impl Draft {
    fn add_line(&mut self) {
        self.next += 1;
        self.lines.push(Line {
            id: self.next,
            ..Line::default()
        });
    }
}

pub(super) struct BillForm {
    pages: Pages,
    note: bool,
    /// The posted bill a debit note takes back from.
    for_bill: Option<i64>,
    /// The draft this form replaces when saved.
    correcting: Option<i64>,
    source: Option<Document>,
    draft: Draft,
    started: bool,
    errors: Vec<(String, String)>,
    submitted: bool,
    notice: Option<String>,
    suppliers: Vec<Party>,
    accounts: Vec<Account>,
    taxes: Vec<TaxCode>,
    units: Option<MinorUnits>,
    currency: String,
    unavailable: Option<&'static str>,
}

impl BillForm {
    pub(super) fn new(pages: Pages, note: bool, query: &HashMap<String, String>) -> Result<Self> {
        let id = |key: &str| params::positive_id(query.get(key).map(String::as_str));
        Ok(Self {
            pages,
            note,
            for_bill: if note { id("bill") } else { None },
            correcting: if note { None } else { id("correct") },
            source: None,
            draft: Draft::default(),
            started: false,
            errors: vec![],
            submitted: false,
            notice: None,
            suppliers: vec![],
            accounts: vec![],
            taxes: vec![],
            units: None,
            currency: String::new(),
            unavailable: None,
        })
    }

    pub(super) async fn reload(&mut self, scope: &Scope) -> Result<()> {
        self.unavailable = None;
        let Some(company) = company(scope) else {
            self.unavailable = Some("Choose a company to record documents for.");
            return Ok(());
        };
        let book = match self.pages.ledger.get_book(scope, company).await {
            Ok(book) => book,
            Err(_) => {
                self.unavailable =
                    Some("This company's books are not open yet, or you cannot read them.");
                return Ok(());
            }
        };
        self.units = Some(book.minor_units);
        self.currency = book.currency_code;
        self.accounts = self
            .pages
            .ledger
            .list_accounts(scope, company)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|a| {
                a.active && matches!(a.account_type, AccountType::Asset | AccountType::Expense)
            })
            .collect();
        self.taxes = self
            .pages
            .payables
            .list_tax_codes(scope, company)
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| t.active)
            .collect();
        if self.started {
            return Ok(());
        }
        let wanted = self.for_bill.or(self.correcting);
        if self.note && self.for_bill.is_none() {
            self.unavailable = Some("A debit note is recorded from a posted bill.");
            return Ok(());
        }
        if let Some(id) = wanted {
            self.source = self
                .pages
                .payables
                .get_document(scope, company, id)
                .await
                .ok();
            let usable = self.source.as_ref().is_some_and(|d| {
                d.document_type == "bill"
                    && if self.note {
                        d.status == "posted"
                    } else {
                        d.status == "draft" && corrigible(d)
                    }
            });
            if !usable {
                self.unavailable = Some(if self.note {
                    "Only a posted bill can be corrected with a debit note."
                } else {
                    "That draft can no longer be corrected here."
                });
                return Ok(());
            }
        }
        if !self.note {
            self.suppliers = self.recent_suppliers(scope, company).await;
        }
        self.start(scope).await?;
        Ok(())
    }

    /// The parties of the newest bills, newest first, one entry each.
    async fn recent_suppliers(&self, scope: &Scope, company: i64) -> Vec<Party> {
        let page = self
            .pages
            .payables
            .page_documents(
                scope,
                company,
                &DocumentPageQuery {
                    document_type: Some("bill".into()),
                    page_size: 100,
                    ..DocumentPageQuery::default()
                },
            )
            .await;
        let mut seen = std::collections::HashSet::new();
        page.map(|p| p.documents)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| seen.insert(d.party.key.clone()))
            .map(|d| d.party)
            .collect()
    }

    /// The first values of the form: today, or the draft being corrected.
    async fn start(&mut self, scope: &Scope) -> Result<()> {
        let units = self.units.ok_or(Error::Internal)?;
        let mut draft = Draft::default();
        match (&self.source, self.note) {
            (Some(d), false) => {
                draft.key = d.party.key.clone();
                draft.name = d.party.name.clone();
                draft.tax_id = d.party.tax_id.clone().unwrap_or_default();
                draft.registration = d.party.registration_id.clone().unwrap_or_default();
                draft.address = d.party.address.clone().unwrap_or_default();
                draft.number = d.party_document_number.clone().unwrap_or_default();
                draft.date = d.document_date.to_string();
                draft.due = d.due_date.to_string();
                draft.description = d.description.clone().unwrap_or_default();
                for l in &d.lines {
                    draft.add_line();
                    let line = draft.lines.last_mut().ok_or(Error::Internal)?;
                    line.description = l.description.clone();
                    line.account = l.account_id.to_string();
                    line.tax = l.tax_code_id.map(|t| t.to_string()).unwrap_or_default();
                    line.amount = l.amount.shown(units);
                }
            }
            _ => {
                draft.date = self.pages.today(scope).await?.to_string();
                draft.add_line();
            }
        }
        self.draft = draft;
        self.started = true;
        Ok(())
    }

    /// Reads the browser's fields into the draft.
    fn read(&mut self, v: &Value) {
        let mut fields: HashMap<&str, &str> = HashMap::new();
        let mut lines: BTreeMap<u32, Line> = BTreeMap::new();
        let mut any_line = false;
        for (name, value) in v.as_object().into_iter().flatten() {
            let Some(key) = name
                .strip_prefix(PREFIX)
                .and_then(|n| n.strip_prefix('['))
                .and_then(|n| n.strip_suffix(']'))
            else {
                continue;
            };
            let Some(text) = value.as_str() else { continue };
            if let Some((id, field)) = key
                .strip_prefix("line_")
                .and_then(|k| k.split_once('_'))
                .and_then(|(id, field)| Some((id.parse::<u32>().ok()?, field)))
            {
                any_line = true;
                let line = lines.entry(id).or_insert_with(|| Line {
                    id,
                    ..Line::default()
                });
                match field {
                    "description" => line.description = text.into(),
                    "account" => line.account = text.into(),
                    "tax" => line.tax = text.into(),
                    "amount" => line.amount = text.into(),
                    _ => {}
                }
            } else {
                fields.insert(key, text);
            }
        }
        let d = &mut self.draft;
        let take = |fields: &HashMap<&str, &str>, key: &str, into: &mut String| {
            if let Some(text) = fields.get(key) {
                *into = (*text).into();
            }
        };
        take(&fields, "key", &mut d.key);
        take(&fields, "name", &mut d.name);
        take(&fields, "tax_id", &mut d.tax_id);
        take(&fields, "registration", &mut d.registration);
        take(&fields, "address", &mut d.address);
        take(&fields, "number", &mut d.number);
        take(&fields, "date", &mut d.date);
        take(&fields, "due", &mut d.due);
        take(&fields, "description", &mut d.description);
        if any_line {
            d.lines = lines.into_values().collect();
            d.next = d.next.max(d.lines.last().map_or(0, |l| l.id));
        }
        // Choosing a supplier fills the fields once; editing them afterwards
        // does not choose again.
        if let Some(pick) = fields.get("pick") {
            if *pick != d.pick {
                d.pick = (*pick).into();
                if let Some(p) = self.suppliers.iter().find(|p| p.key == *pick) {
                    d.key = p.key.clone();
                    d.name = p.name.clone();
                    d.tax_id = p.tax_id.clone().unwrap_or_default();
                    d.registration = p.registration_id.clone().unwrap_or_default();
                    d.address = p.address.clone().unwrap_or_default();
                }
            }
        }
    }

    /// The contract's input, leaving out what the person left blank so its
    /// own words say what is missing.
    fn input(&self, company: i64) -> Value {
        let d = &self.draft;
        let mut v = json!({"company_id": company});
        if let Some(bill) = self.for_bill {
            v["bill_id"] = json!(bill);
        } else {
            put(&mut v, "party_key", &d.key);
            put(&mut v, "party_name", &d.name);
            put(&mut v, "party_tax_id", &d.tax_id);
            put(&mut v, "party_registration_id", &d.registration);
            put(&mut v, "party_address", &d.address);
        }
        put(&mut v, "party_document_number", &d.number);
        put(&mut v, "document_date", &d.date);
        put(&mut v, "due_date", &d.due);
        put(&mut v, "description", &d.description);
        v["lines"] = json!(d
            .lines
            .iter()
            .map(|l| {
                let mut line = json!({});
                if !l.description.trim().is_empty() {
                    line["description"] = json!(l.description.trim());
                }
                if let Ok(id) = l.account.trim().parse::<i64>() {
                    line["account_id"] = json!(id);
                }
                if let Ok(id) = l.tax.trim().parse::<i64>() {
                    line["tax_code_id"] = json!(id);
                }
                if !l.amount.trim().is_empty() {
                    line["amount"] = json!(l.amount.trim());
                }
                line
            })
            .collect::<Vec<_>>());
        v
    }

    /// Net, tax by code and total as the facade will work them out: tax is
    /// rounded half up once per code.
    fn totals(&self) -> Option<(Money, Money, Money, bool)> {
        let units = self.units?;
        let mut net = Decimal::ZERO;
        let mut bases: Vec<(i64, Decimal)> = vec![];
        let mut complete = true;
        for l in &self.draft.lines {
            if l.amount.trim().is_empty() {
                complete = false;
                continue;
            }
            let Ok(amount) = Money::parse(&l.amount, units) else {
                complete = false;
                continue;
            };
            net += amount.amount();
            if let Ok(code) = l.tax.trim().parse::<i64>() {
                match bases.iter_mut().find(|(c, _)| *c == code) {
                    Some((_, base)) => *base += amount.amount(),
                    None => bases.push((code, amount.amount())),
                }
            }
        }
        let mut tax = Decimal::ZERO;
        for (code, base) in bases {
            if let Some(t) = self.taxes.iter().find(|t| t.id == code) {
                tax += (base * t.rate / Decimal::from(100)).round_dp_with_strategy(
                    u32::from(units.get()),
                    RoundingStrategy::MidpointAwayFromZero,
                );
            }
        }
        let money = |d: Decimal| Money::new(d, units).ok();
        Some((money(net)?, money(tax)?, money(net + tax)?, complete))
    }

    fn problems(&mut self, fields: BTreeMap<String, Vec<String>>) {
        self.errors = fields
            .into_iter()
            .flat_map(|(path, messages)| messages.into_iter().map(move |m| (path.clone(), m)))
            .collect();
    }

    /// The element a contract path belongs to and the name its sentence uses.
    fn locate(&self, path: &str) -> Option<(String, String)> {
        let top = |id: &str, label: &str| Some((format!("payables-record-{id}"), label.to_owned()));
        match path {
            "party_key" => top("key", "Supplier code"),
            "party_name" => top("name", "Supplier name"),
            "party_tax_id" => top("tax-id", "Tax ID"),
            "party_registration_id" => top("registration", "Registration number"),
            "party_address" => top("address", "Address"),
            "party_document_number" => top("number", "Supplier's invoice number"),
            "document_date" => top("date", "Date"),
            "due_date" => top("due", "Due date"),
            "description" => top("description", "Description"),
            "lines" => Some(("payables-record-add-line".into(), "Lines".into())),
            _ => {
                let rest = path.strip_prefix("lines.")?;
                let (index, field) = rest.split_once('.')?;
                let line = self.draft.lines.get(index.parse::<usize>().ok()?)?;
                let (element, label) = match field {
                    "description" => ("description", "description"),
                    "account_id" => ("account", "account"),
                    "tax_code_id" => ("tax", "tax code"),
                    "amount" => ("amount", "amount"),
                    _ => return None,
                };
                Some((
                    format!("payables-record-line-{}-{element}", line.id),
                    format!("Line {} {label}", index.parse::<usize>().ok()? + 1),
                ))
            }
        }
    }

    fn errors_for(&self, element: &str) -> Vec<String> {
        self.errors
            .iter()
            .filter_map(|(path, message)| {
                let (id, label) = self.locate(path)?;
                (id == element).then(|| error_sentence(&label, message))
            })
            .collect()
    }

    async fn save(&mut self, scope: &Scope, credential: &str) -> Result<Option<String>> {
        self.submitted = true;
        self.errors.clear();
        self.notice = None;
        let Some(company) = company(scope) else {
            self.notice = Some("Choose a company first.".into());
            return Ok(None);
        };
        // A draft cannot be edited: the corrected one replaces it, so the
        // old draft is voided first (the supplier's invoice number is only
        // free once it is).
        if let Some(old) = self.correcting {
            let voided = self
                .pages
                .write(
                    scope,
                    credential,
                    "save",
                    "finance.payables.document.void",
                    json!({"company_id": company, "document_id": old, "void_reason": "Replaced by a corrected draft."}),
                )
                .await?;
            match voided {
                Written::Done(_) => self.correcting = None,
                Written::Forbidden => {
                    self.notice = Some(
                        "You do not have permission to void the draft this one replaces.".into(),
                    );
                    return Ok(None);
                }
                Written::Fields(_) | Written::Changed => {
                    self.notice = Some("The draft can no longer be replaced: it was approved or voided while you were editing. Open it again to see where it stands.".into());
                    return Ok(None);
                }
            }
        }
        let contract = if self.note {
            "finance.payables.debit_note.create"
        } else {
            "finance.payables.bill.create"
        };
        let written = self
            .pages
            .write(scope, credential, "save", contract, self.input(company))
            .await?;
        match written {
            Written::Done(result) => {
                let id = record_id(&result);
                Ok(id.map(|id| format!("{INDEX}/{id}")))
            }
            Written::Forbidden => {
                self.notice = Some(format!(
                    "You do not have permission to record {}.",
                    if self.note { "debit notes" } else { "bills" }
                ));
                Ok(None)
            }
            Written::Changed => {
                self.notice = Some(
                    "Nothing was saved because something changed. Check the form and save again."
                        .into(),
                );
                Ok(None)
            }
            Written::Fields(fields) => {
                self.problems(fields);
                if self.errors.iter().any(|(path, _)| path == "base")
                    || self.errors.iter().all(|(p, _)| self.locate(p).is_none())
                {
                    let loose: Vec<_> = self
                        .errors
                        .iter()
                        .filter(|(p, _)| self.locate(p).is_none())
                        .map(|(_, m)| m.as_str())
                        .collect();
                    self.notice = Some(loose.join(" "));
                }
                if self.notice.is_none() && self.errors.is_empty() {
                    self.notice =
                        Some("The document was not saved. Check the form and try again.".into());
                }
                if self.for_bill.is_none() && self.correcting.is_none() && self.source.is_some() {
                    self.notice = Some(format!(
                        "The old draft is already voided. {}",
                        self.notice.take().unwrap_or_default()
                    ));
                }
                Ok(None)
            }
        }
    }

    pub(super) fn body(&self) -> Result<String> {
        let title = if self.note {
            "Record a debit note"
        } else if self.source.is_some() {
            "Correct a draft bill"
        } else {
            "Record a bill"
        };
        let back = render(BackLink {
            href: INDEX,
            id: "payables-record-back",
            title: "Back to bills",
            class: "",
        })?;
        if let Some(why) = self.unavailable {
            let mut empty = EmptyState::missing("Nothing to record here", why);
            empty.id = "payables-record-unavailable".into();
            return Unavailable {
                title,
                back,
                empty: render(empty)?,
            }
            .render()
            .map_err(|_| Error::Internal);
        }
        let units = self.units.ok_or(Error::Internal)?;
        let d = &self.draft;
        let field = |key: &str, element: &str, label: &str, value: &str| {
            Field::in_form(PREFIX, key)
                .id(format!("payables-record-{element}"))
                .label(label)
                .value(value)
                .errors(self.errors_for(&format!("payables-record-{element}")))
        };
        let suppliers = if self.suppliers.is_empty() {
            None
        } else {
            Some(
                Field::in_form(PREFIX, "pick")
                    .id("payables-record-pick")
                    .label("Supplier you billed before")
                    .kind(InputKind::Select)
                    .prompt("Choose a supplier to fill in the details")
                    .value(d.pick.clone())
                    .options(
                        self.suppliers
                            .iter()
                            .map(|p| Choice::new(format!("{} · {}", p.key, p.name), p.key.clone()))
                            .collect(),
                    ),
            )
        };
        let party: Vec<Field> = if self.note {
            vec![]
        } else {
            vec![
                field("key", "key", "Supplier code", &d.key)
                    .required()
                    .hint("Your own short code for the supplier, such as SUP-001."),
                field("name", "name", "Supplier name", &d.name).required(),
                field("tax_id", "tax-id", "Tax ID", &d.tax_id),
                field(
                    "registration",
                    "registration",
                    "Registration number",
                    &d.registration,
                ),
                field("address", "address", "Address", &d.address)
                    .kind(InputKind::Textarea)
                    .wrapper_class("md:col-span-2"),
            ]
        };
        let source_facts = match (&self.source, self.note) {
            (Some(b), true) => Some(render(Facts {
                id: "payables-record-bill-facts",
                items: vec![
                    plain("Supplier", &format!("{} · {}", b.party.key, b.party.name)),
                    plain(
                        "Bill",
                        &format!(
                            "{} · {}",
                            f::number(b.document_number),
                            f::optional(&b.party_document_number)
                        ),
                    ),
                    plain("Bill date", &b.document_date.to_string()),
                    plain(
                        "Still owed",
                        &b.outstanding.map_or_else(
                            || "—".into(),
                            |m| format!("{} {}", f::money(m, units), b.currency_code),
                        ),
                    ),
                ],
            })?),
            _ => None,
        };
        let mut document = vec![
            field(
                "number",
                "number",
                if self.note {
                    "Supplier's credit note number"
                } else {
                    "Supplier's invoice number"
                },
                &d.number,
            ),
            field("date", "date", "Date", &d.date)
                .kind(InputKind::Date)
                .required(),
            field("due", "due", "Due date", &d.due)
                .kind(InputKind::Date)
                .hint("Left empty, it is the date."),
            field("description", "description", "Description", &d.description),
        ];
        if !self.note {
            document[0] = document[0].clone().required();
        }
        let account_choices: Vec<Choice> = self
            .accounts
            .iter()
            .map(|a| Choice::new(format!("{} {}", a.code, a.name), a.id.to_string()))
            .collect();
        let tax_choices: Vec<Choice> = self
            .taxes
            .iter()
            .map(|t| {
                Choice::new(
                    format!("{} · {} ({}%)", t.code, t.name, t.rate.normalize()),
                    t.id.to_string(),
                )
            })
            .collect();
        let mut lines = Vec::new();
        for (index, l) in d.lines.iter().enumerate() {
            let at = |part: &str| format!("payables-record-line-{}-{part}", l.id);
            let line_field = |part: &str, key: &str, label: &str, value: &str| {
                Field::in_form(PREFIX, &format!("line_{}_{key}", l.id))
                    .id(at(part))
                    .label(label)
                    .value(value)
                    .errors(self.errors_for(&at(part)))
            };
            lines.push(LineView {
                number: index + 1,
                dom_id: format!("payables-record-line-{}", l.id),
                description: line_field(
                    "description",
                    "description",
                    "What it is for",
                    &l.description,
                )
                .required(),
                account: Combobox::new(at("account"), format!("{PREFIX}[line_{}_account]", l.id))
                    .label("Account")
                    .value(l.account.clone())
                    .options(account_choices.clone())
                    .placeholder("Search accounts")
                    .errors(self.errors_for(&at("account"))),
                tax: line_field("tax", "tax", "Tax code", &l.tax)
                    .kind(InputKind::Select)
                    .prompt("No tax")
                    .options(tax_choices.clone()),
                amount: line_field("amount", "amount", "Amount", &l.amount)
                    .attr("inputmode", "decimal")
                    .attr("data-debounce", "400")
                    .required(),
                remove: if d.lines.len() > 1 {
                    Some(render(Button {
                        id: &at("remove"),
                        button_type: "button",
                        event: "remove_line",
                        name: "line",
                        value: &l.id.to_string(),
                        body: Html::text("Remove"),
                        ..Button::default()
                    })?)
                } else {
                    None
                },
            });
        }
        let add_line = if d.lines.len() < MAX_LINES {
            Some(render(AddMore::new(
                "payables-record-add-line",
                "Add a line",
                "add_line",
            ))?)
        } else {
            None
        };
        let totals = match self.totals() {
            Some((net, tax, total, complete)) => {
                let shown = |m: Money| format!("{} {}", f::money(m, units), self.currency);
                Some(render(Facts {
                    id: "payables-record-totals",
                    items: vec![
                        plain("Net", &shown(net)),
                        plain("Tax", &shown(tax)),
                        plain(
                            if complete { "Total" } else { "Total so far" },
                            &shown(total),
                        ),
                    ],
                })?)
            }
            None => None,
        };
        let summary = if self.submitted && !self.errors.is_empty() {
            let mut summary = ErrorSummary::new("payables-record-errors");
            for (path, message) in &self.errors {
                if let Some((element, label)) = self.locate(path) {
                    summary.push(element, error_sentence(&label, message));
                }
            }
            Some(render(summary)?)
        } else {
            None
        };
        let notice = self
            .notice
            .as_ref()
            .map(|message| {
                render(Alert {
                    kind: StatusKind::Error,
                    id: "payables-record-notice",
                    body: Html::text(message),
                    ..Alert::default()
                })
            })
            .transpose()?;
        let replaces = match (&self.source, self.note, self.correcting) {
            (Some(old), false, Some(_)) => Some(format!(
                "Saving replaces draft #{}: it is voided and this version is recorded in its place.",
                old.id
            )),
            _ => None,
        };
        let no_accounts = self.accounts.is_empty().then_some(
            "No accounts to choose from. Accounts come from the company's chart, and need permission to read the ledger.",
        );
        let save = render(Button {
            id: "payables-record-save",
            button_type: "submit",
            variant: ButtonVariant::Primary,
            working: "Saving…",
            body: Html::text("Save draft"),
            ..Button::default()
        })?;
        View {
            title,
            subtitle: if self.note {
                "Takes back part of a posted bill. It stays a draft until someone approves and posts it."
            } else {
                "It stays a draft until someone approves and posts it from its page."
            },
            back,
            summary,
            notice,
            replaces,
            suppliers,
            party,
            source_facts,
            document,
            lines,
            add_line,
            no_accounts,
            totals,
            save,
            party_heading: if self.note { "Bill" } else { "Supplier" },
            document_heading: if self.note { "Debit note" } else { "Invoice" },
        }
        .render()
        .map_err(|_| Error::Internal)
    }

    async fn event(&mut self, scope: &Scope, credential: &str, v: &Value) -> Result<LiveBody> {
        let kind = v["type"].as_str().ok_or(Error::Invalid)?;
        let mut redirect = None;
        match kind {
            "edit" => {
                self.read(v);
                self.errors.clear();
                self.notice = None;
            }
            "add_line" => {
                self.errors.clear();
                if self.draft.lines.len() < MAX_LINES {
                    self.draft.add_line();
                }
            }
            "remove_line" => {
                self.errors.clear();
                if let Some(id) = v["line"].as_str().and_then(|s| s.parse::<u32>().ok()) {
                    if self.draft.lines.len() > 1 {
                        self.draft.lines.retain(|l| l.id != id);
                    }
                }
            }
            "save" => {
                let needed = if self.correcting.is_some() {
                    [CREATE_DOCUMENTS, APPROVE_DOCUMENTS].to_vec()
                } else {
                    [CREATE_DOCUMENTS].to_vec()
                };
                self.read(v);
                let mut allowed = true;
                for capability in needed {
                    allowed &= self.pages.allowed(scope, capability).await?;
                }
                if !allowed {
                    self.notice =
                        Some("You do not have permission to record this document.".into());
                } else {
                    redirect = self.save(scope, credential).await?;
                }
            }
            _ => return Err(Error::Invalid),
        }
        Ok(match redirect {
            Some(to) => LiveBody {
                redirect: Some(to),
                flash: Some((
                    StatusKind::Success,
                    format!(
                        "{} saved as a draft. Approve and post it when it is ready.",
                        if self.note { "Debit note" } else { "Bill" }
                    ),
                )),
                ..LiveBody::default()
            },
            None => LiveBody {
                html: self.body()?,
                ..LiveBody::default()
            },
        })
    }
}

impl LiveState<Scope, Error> for BillForm {
    fn update<'a>(&'a mut self, r: LiveRequest<'a, Scope>) -> LiveFuture<'a, Error> {
        Box::pin(async move {
            match r.input {
                LiveInput::Refresh | LiveInput::Params(_) => {
                    self.reload(r.scope).await?;
                    Ok(LiveBody {
                        html: self.body()?,
                        ..LiveBody::default()
                    })
                }
                LiveInput::Event(v) => self.event(r.scope, r.credential, v).await,
            }
        })
    }
}

/// A draft that names order lines cannot be corrected here: this form has no
/// place for them and the corrected one would lose them.
pub(super) fn corrigible(d: &Document) -> bool {
    d.lines
        .iter()
        .all(|l| l.order_line_id.is_none() && l.external_order_line_id.is_none())
}

/// Sets `key` unless the person left the field blank.
fn put(v: &mut Value, key: &str, text: &str) {
    if !text.trim().is_empty() {
        v[key] = json!(text.trim());
    }
}

fn plain(title: &'static str, value: &str) -> Fact<'static> {
    Fact {
        title,
        value: Html::text(value),
        ..Fact::default()
    }
}

struct LineView {
    number: usize,
    dom_id: String,
    description: Field,
    account: Combobox,
    tax: Field,
    amount: Field,
    remove: Option<Html>,
}

#[derive(Template)]
#[template(path = "bill_form.html")]
struct View {
    title: &'static str,
    subtitle: &'static str,
    back: Html,
    summary: Option<Html>,
    notice: Option<Html>,
    replaces: Option<String>,
    suppliers: Option<Field>,
    party: Vec<Field>,
    source_facts: Option<Html>,
    document: Vec<Field>,
    lines: Vec<LineView>,
    add_line: Option<Html>,
    no_accounts: Option<&'static str>,
    totals: Option<Html>,
    save: Html,
    party_heading: &'static str,
    document_heading: &'static str,
}

#[derive(Template)]
#[template(
    source = r#"{% import "components.html" as ui %}{% call ui::page(id="payables-record", variant=bilibi_ui::PageVariant::Form) %}{% call ui::header(actions=back) %}{{ title }}{% endcall %}{{ empty|safe }}{% endcall %}"#,
    ext = "html"
)]
struct Unavailable {
    title: &'static str,
    back: Html,
    empty: Html,
}
