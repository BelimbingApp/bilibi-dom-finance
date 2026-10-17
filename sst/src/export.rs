//! Bilimbi.Finance.Sst.Export: a return as CSV in the SST-02 item order, and
//! as a printable HTML document. Both name the return's status, so a draft is
//! never mistaken for what was finalised; every text value is escaped for
//! its format. The document is a file the company keeps or prints, not a
//! page of the app, so it carries its own print styles.
use crate::form::{self, BoxKind};
use crate::returns::{ReturnReport, ReturnStatus};
use crate::*;
use rust_decimal::Decimal;

/// The two shapes a return is exported in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Csv,
    Html,
}

/// An exported return: its file name, media type and content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub filename: String,
    pub content_type: &'static str,
    pub content: String,
}

fn tax_name(tax_type: TaxType) -> &'static str {
    match tax_type {
        TaxType::SalesTax => "Sales tax",
        TaxType::ServiceTax => "Service tax",
    }
}

pub(crate) fn render(report: &ReturnReport, company: &str, format: ExportFormat) -> Result<Export> {
    let figures = report.figures.as_ref().ok_or(SstError::ReturnDiscarded)?;
    let r = &report.tax_return;
    let filename = |extension: &str| {
        format!(
            "sst-02-{}-{}-{}-return-{}.{extension}",
            r.tax_type.as_str().replace('_', "-"),
            r.period_start,
            r.period_end,
            r.return_number
        )
    };
    Ok(match format {
        ExportFormat::Csv => Export {
            filename: filename("csv"),
            content_type: "text/csv",
            content: csv(report, figures, company),
        },
        ExportFormat::Html => Export {
            filename: filename("html"),
            content_type: "text/html",
            content: html(report, figures, company),
        },
    })
}

fn status(report: &ReturnReport) -> String {
    match (report.tax_return.status, report.tax_return.version) {
        (ReturnStatus::Finalised, Some(version)) => format!("Finalised, version {version}"),
        _ => "Draft, not finalised".into(),
    }
}

/// Rounded half up to the book's minor units and written with exactly that
/// many places; blank where the box takes no such column.
fn amount(amount: Option<Decimal>, minor_units: u8) -> String {
    amount.map_or_else(String::new, |a| {
        let mut rounded = form::round(a, u32::from(minor_units));
        rounded.rescale(u32::from(minor_units));
        rounded.to_string()
    })
}

fn csv(report: &ReturnReport, figures: &Figures, company: &str) -> String {
    let r = &report.tax_return;
    let units = figures.minor_units;
    let row = |cells: [&str; 5]| cells.map(str::to_owned).to_vec();
    let mut rows = vec![
        row(["part", "item", "description", "value", "tax"]),
        row(["A", "", "Status", &status(report), ""]),
        row([
            "A",
            "",
            "Amendment",
            if r.amends_return_id.is_some() {
                "yes"
            } else {
                "no"
            },
            "",
        ]),
        row(["A", "1", "Type of return", tax_name(r.tax_type), ""]),
        row([
            "A",
            "1",
            "SST registration number",
            report.registration_number.as_deref().unwrap_or(""),
            "",
        ]),
        row(["A", "2", "Name", company, ""]),
        row([
            "A",
            "3",
            "Taxable period from",
            &r.period_start.to_string(),
            "",
        ]),
        row(["A", "3", "Taxable period to", &r.period_end.to_string(), ""]),
        row([
            "A",
            "4",
            "Return and payment due date",
            &r.due_date.to_string(),
            "",
        ]),
        row(["A", "", "Currency", &figures.currency_code, ""]),
    ];
    rows.extend(figures.boxes.iter().map(|b| {
        row([
            b.part,
            b.item,
            b.label,
            &amount(b.taxable_amount, units),
            &amount(b.tax_amount, units),
        ])
    }));
    rows.push(row([
        "",
        "",
        "Read through ledger entry",
        &figures.through_entry_number.to_string(),
        "",
    ]));
    rows.push(row([
        "",
        "",
        "Read through mapping",
        &figures.through_mapping_number.to_string(),
        "",
    ]));
    rows.iter()
        .map(|cells| cells.iter().map(|c| cell(c)).collect::<Vec<_>>().join(",") + "\r\n")
        .collect()
}

/// RFC 4180 quoting, and a leading quote on text a spreadsheet would read as
/// a formula; amounts are written as plain numbers.
fn cell(text: &str) -> String {
    let number = {
        let digits = text.strip_prefix('-').unwrap_or(text);
        let (whole, fraction) = digits.split_once('.').unwrap_or((digits, "1"));
        [whole, fraction]
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    };
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) && !number {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// Text escaped for HTML content and attribute values.
fn e(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            c => escaped.push(c),
        }
    }
    escaped
}

fn html(report: &ReturnReport, figures: &Figures, company: &str) -> String {
    let r = &report.tax_return;
    let units = figures.minor_units;
    let tax = tax_name(r.tax_type);
    let rows = figures
        .boxes
        .iter()
        .map(|b| {
            format!(
                "<tr class=\"{}\"><td>{}</td><td>{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                if b.kind == BoxKind::Total { "total" } else { "" },
                e(b.item),
                e(b.label),
                e(&amount(b.taxable_amount, units)),
                e(&amount(b.tax_amount, units))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let adjustments = report
        .adjustments
        .iter()
        .filter(|a| a.standing)
        .map(|a| {
            let item = form::find(r.tax_type, &a.box_key).map_or("", |b| b.item);
            format!(
                "<tr><td>{}</td><td>{}{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                e(item),
                e(&a.description),
                a.reference
                    .as_deref()
                    .map_or_else(String::new, |r| format!(" ({})", e(r))),
                e(&amount(Some(a.taxable_amount), units)),
                e(&amount(Some(a.tax_amount), units))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let accounts = figures
        .accounts
        .iter()
        .map(|a| {
            format!(
                "<tr><td>{} {}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                e(&a.account_code),
                e(&a.account_name),
                e(&amount(Some(a.ledger_credits - a.ledger_debits), units)),
                e(&amount(Some(a.document_credits - a.document_debits), units)),
                e(&amount(Some(a.difference), units))
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let state = if r.status == ReturnStatus::Finalised {
        format!(
            "<p>{}, read through ledger entry {} and mapping {}.</p>",
            e(&status(report)),
            figures.through_entry_number,
            figures.through_mapping_number
        )
    } else {
        "<p class=\"draft\">Draft: not finalised. Its figures change as documents post.</p>".into()
    };
    let none = |rows: String, empty: &str| {
        if rows.is_empty() {
            format!("<tr><td colspan=\"4\">{empty}</td></tr>")
        } else {
            rows
        }
    };
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>SST-02 {tax} {start} to {end}</title>
<style>
body {{ font: 12px/1.4 system-ui, sans-serif; color: #111; margin: 24px; }}
h1 {{ font-size: 18px; margin: 0 0 4px; }}
h2 {{ font-size: 14px; margin: 20px 0 6px; }}
table {{ border-collapse: collapse; width: 100%; }}
th, td {{ border: 1px solid #999; padding: 4px 6px; text-align: left; vertical-align: top; }}
td.n, th.n {{ text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }}
tr.total td {{ font-weight: 600; background: #f2f2f2; }}
.draft {{ border: 2px solid #b00; color: #b00; padding: 6px 10px; display: inline-block; font-weight: 600; }}
dl {{ display: grid; grid-template-columns: max-content 1fr; gap: 2px 12px; margin: 0; }}
dt {{ color: #555; }}
@media print {{ body {{ margin: 0; }} }}
</style>
</head>
<body>
<h1>SST-02: {tax} return</h1>
{state}
<h2>Part A</h2>
<dl>
<dt>1. Type of return</dt><dd>{tax}{amendment}</dd>
<dt>1. SST registration number</dt><dd>{number}</dd>
<dt>2. Name</dt><dd>{company}</dd>
<dt>3. Taxable period</dt><dd>{start} to {end}</dd>
<dt>4. Return and payment due date</dt><dd>{due}</dd>
<dt>Currency</dt><dd>{currency}</dd>
</dl>
<h2>Items</h2>
<table>
<thead><tr><th>Item</th><th>Description</th><th class="n">Value</th><th class="n">Tax</th></tr></thead>
<tbody>
{rows}
</tbody>
</table>
<h2>Adjustments</h2>
<table>
<thead><tr><th>Item</th><th>Description</th><th class="n">Value</th><th class="n">Tax</th></tr></thead>
<tbody>
{adjustments}
</tbody>
</table>
<h2>Ledger reconciliation</h2>
<table>
<thead><tr><th>Tax account</th><th class="n">Ledger, credits less debits</th><th class="n">Documents</th><th class="n">Difference</th></tr></thead>
<tbody>
{accounts}
</tbody>
</table>
<p>Items 15 (penalty) and 16, Part B1's item lines, and Part F's declaration are completed when the return is filed.</p>
</body>
</html>
"#,
        tax = e(tax),
        start = r.period_start,
        end = r.period_end,
        due = r.due_date,
        amendment = if r.amends_return_id.is_some() {
            " (amendment)"
        } else {
            ""
        },
        number = e(report.registration_number.as_deref().unwrap_or("")),
        company = e(company),
        currency = e(&figures.currency_code),
        adjustments = none(adjustments, "None."),
        accounts = none(accounts, "No tax account."),
    )
}

impl Sst {
    /// The return as CSV in the SST-02 item order, preceded by Part A and its
    /// status (a draft until finalised, then its version) and followed by the
    /// numbers it was read at; or, as `ExportFormat::Html`, a printable
    /// document with Part A, the items, the standing adjustments and the
    /// ledger reconciliation. A discarded return is `ReturnDiscarded`. An open
    /// return's figures are read now, so it also needs the receivables,
    /// payables and ledger view capabilities.
    pub async fn export_return(
        &self,
        s: &Scope,
        company: i64,
        id: i64,
        format: ExportFormat,
    ) -> Result<Export> {
        let (name, report) = self.export_report(s, company, id).await?;
        render(&report, &name, format)
    }
}
