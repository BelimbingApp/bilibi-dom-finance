//! BillLive and BillsLive's labels, numeric normalization and refusal copy.
use crate::{Document, MatchEvent, PayablesError, Tolerance, ToleranceKind};
use askama::Template;
use bilibi_database::{Error, Result};
use bilibi_datetime::{DisplayContext, Format, Precision};
use bilibi_finance_ledger::{LedgerError, MinorUnits, Money};
use bilibi_ui::{Badge, BadgeKind, Html};
use rust_decimal::Decimal;

pub(super) fn kind(document_type: &str) -> &'static str {
    if document_type == "bill" {
        "Bill"
    } else {
        "Debit note"
    }
}
pub(super) fn number(number: Option<i64>) -> String {
    number.map_or_else(|| "—".into(), |n| format!("#{n}"))
}
pub(super) fn decimal(n: Option<Decimal>) -> String {
    n.map_or_else(|| "—".into(), |n| n.normalize().to_string())
}
pub(super) fn optional(text: &Option<String>) -> &str {
    text.as_deref().unwrap_or("—")
}
pub(super) fn actor(user: Option<i64>, impersonator: Option<i64>) -> String {
    match (user, impersonator) {
        (Some(u), Some(i)) => format!("User #{u} (impersonated by #{i})"),
        (Some(u), None) => format!("User #{u}"),
        (None, _) => "—".into(),
    }
}
pub(super) fn status(status: &str) -> Result<Html> {
    badge(
        status,
        match status {
            "posted" => BadgeKind::Success,
            "held" => BadgeKind::Warning,
            "voided" => BadgeKind::Danger,
            _ => BadgeKind::Neutral,
        },
        true,
    )
}
pub(super) fn match_label(status: &str) -> &'static str {
    match status {
        "pending" => "not yet matched",
        "held" => "held",
        "exception_approved" => "exception approved",
        "matched" => "matched",
        "posted_with_exceptions" => "posted with exceptions",
        _ => "—",
    }
}
pub(super) fn match_badge(status: &str, prefix: &str) -> Result<Html> {
    badge(
        &format!("{prefix}{}", match_label(status)),
        match status {
            "held" | "posted_with_exceptions" => BadgeKind::Warning,
            "matched" => BadgeKind::Success,
            _ => BadgeKind::Neutral,
        },
        true,
    )
}
pub(super) fn badge(text: &str, kind: BadgeKind, dot: bool) -> Result<Html> {
    super::render(Badge {
        kind,
        dot,
        class: "",
        body: Html::text(text),
    })
}
pub(super) fn last_match(d: &Document) -> Option<&MatchEvent> {
    d.match_events
        .iter()
        .rev()
        .find(|e| e.event != "exception_approved")
}
pub(super) fn exceptions(e: &MatchEvent) -> usize {
    e.lines.iter().map(|l| l.exceptions.len()).sum()
}
pub(super) fn tolerance(t: &Option<Tolerance>) -> String {
    match t {
        None => "exact".into(),
        Some(t) => format!(
            "{}{}",
            t.value.normalize(),
            if t.kind == ToleranceKind::Percent {
                "%"
            } else {
                ""
            }
        ),
    }
}

// Payables' PostgreSQL JSON timestamps have no offset; schema stamps UTC.
// Never interpret them in the browser's zone or silently replace a bad value.
pub(super) fn time(display: &DisplayContext, id: &str, at: &str) -> Result<Html> {
    let at = at
        .parse::<jiff::civil::DateTime>()
        .map_err(|_| Error::Internal)?
        .to_zoned(jiff::tz::TimeZone::UTC)
        .map_err(|_| Error::Internal)?
        .timestamp();
    let r = display.render(at, Format::DateTime, Precision::Second);
    super::render(Time {
        element_id: id,
        iso: r.iso,
        text: r.text,
        company: r.company,
        utc: r.utc,
        mode: r.mode.as_str(),
    })
}

#[derive(Template)]
#[template(
    source = r#"<time id="{{ element_id }}" datetime="{{ iso }}" data-format="datetime" data-precision="second" data-mode="{{ mode }}" data-text-company="{{ company }}" data-text-utc="{{ utc }}" data-follow-shell="true" data-hook="DateTime" class="tabular-nums">{{ text }}</time>"#,
    ext = "html"
)]
struct Time<'a> {
    element_id: &'a str,
    iso: String,
    text: String,
    company: String,
    utc: String,
    mode: &'static str,
}

pub(super) fn refusal(action: &str, error: PayablesError) -> String {
    use PayablesError::*;
    // The ledger's own access and archive refusals read as the page's
    // (Bilimbi's ledger answers the same `:unauthorized`/`:company_archived`).
    let error = match error {
        Ledger(LedgerError::Unauthorized) => Unauthorized,
        Ledger(LedgerError::CompanyArchived) => CompanyArchived,
        other => other,
    };
    match (action, &error) {
        ("approve", NotDraft) => "Only a draft can be approved.".into(),
        ("approve", Ledger(LedgerError::SamePerson)) => "This company separates duties: someone other than whoever recorded the document must approve it.".into(),
        ("approve", Unauthorized) => "You do not have permission to approve documents.".into(),
        ("post", NotApproved) => "Only an approved or held document can be posted.".into(),
        ("post", Unauthorized) => "You do not have permission to post documents.".into(),
        ("approve" | "post", CompanyArchived) => "This company is archived.".into(),
        ("post", PayablesNotConfigured) => "Set the payable account in the payables settings first.".into(),
        ("post", ExceedsOutstanding) => "The debit note exceeds what is still owed on its bill.".into(),
        ("post", ExceedsBilledQuantity) => "The debit note takes back more than its bill billed on an order line.".into(),
        ("post", Ledger(LedgerError::Invalid(_))) => "The ledger refused the entry; check the accounts.".into(),
        ("approve_exception", NotHeld) => "Only a held bill has exceptions to approve.".into(),
        ("approve_exception", AlreadyApproved) => "These exceptions are already approved.".into(),
        ("approve_exception", StaleHold) => "The bill was matched again since this page loaded. Review the new figures and approve again.".into(),
        ("approve_exception", Ledger(LedgerError::SamePerson)) => "This company separates duties: someone other than whoever recorded the bill or tried to post it must approve its exceptions.".into(),
        ("approve_exception", Unauthorized) => "You do not have permission to approve match exceptions.".into(),
        ("void", NotVoidable) => "Only a draft, approved, or held document can be voided.".into(),
        ("void", Unauthorized) => "You do not have permission to void documents.".into(),
        ("approve_exception" | "void", Invalid(_)) => "Give a reason of 1 to 500 characters.".into(),
        _ => {
            let verb = match action {
                "approve" => "The document could not be approved",
                "post" => "The document could not be posted",
                "void" => "The document could not be voided",
                _ => "The exceptions could not be approved",
            };
            // Database failures may contain SQL or internals; never expose them.
            match error {
                Failed(_) | Ledger(LedgerError::Failed(_)) => format!("{verb}. Please try again."),
                other => format!("{verb} ({other})."),
            }
        }
    }
}

/// Digits in threes: `1234567.50` reads `1,234,567.50`. The text already
/// carries the book's decimal places (`Money::shown`).
pub(super) fn grouped(amount: &str) -> String {
    let (sign, digits) = amount
        .strip_prefix('-')
        .map_or(("", amount), |rest| ("-", rest));
    let (whole, fraction) = digits.split_once('.').map_or((digits, ""), |(w, f)| (w, f));
    let mut out = String::with_capacity(amount.len() + whole.len() / 3);
    for (i, c) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if fraction.is_empty() {
        format!("{sign}{out}")
    } else {
        format!("{sign}{out}.{fraction}")
    }
}
/// An amount of the book, with its decimal places and digit groups.
pub(super) fn money(amount: Money, units: MinorUnits) -> String {
    grouped(&amount.shown(units))
}

pub(super) fn request_status(status: &str) -> Result<Html> {
    let (label, kind) = match status {
        "submitted" => ("waiting for approval", BadgeKind::Neutral),
        "posted" => ("paid", BadgeKind::Success),
        "rejected" => ("rejected", BadgeKind::Danger),
        other => (other, BadgeKind::Neutral),
    };
    badge(label, kind, true)
}

pub(super) fn fact(title: &'static str, value: &str) -> bilibi_ui::Fact<'static> {
    bilibi_ui::Fact {
        title,
        value: Html::text(value),
        ..bilibi_ui::Fact::default()
    }
}

/// Who did it and, when known, when: `User #2 · 2026-10-20 09:30:00`.
pub(super) fn actor_at(
    display: &DisplayContext,
    actor: Option<i64>,
    impersonator: Option<i64>,
    at: Option<&str>,
    element: &str,
    separator: &str,
) -> Result<Html> {
    let mut value = Html::text(&self::actor(actor, impersonator));
    if let Some(at) = at {
        value.push(&Html::text(separator));
        value.push(&time(display, element, at)?);
    }
    Ok(value)
}

/// The owner's refusal in the words a person acts on. Its own wording stays
/// when this page has no better.
pub(super) fn refused(
    action: &str,
    fields: &std::collections::BTreeMap<String, Vec<String>>,
) -> String {
    let message = fields
        .values()
        .flatten()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" ");
    if message.contains("cannot approve it") {
        return "This company separates duties: someone other than whoever recorded it must approve it.".into();
    }
    if message.is_empty() {
        return format!("{action} did not go through. Try again.");
    }
    format!("{action} did not go through: {message}")
}
