//! Shared period selection for the Journals list.
use bilibi_datetime::{Clock, Period, Preset};
use jiff::civil::Date;
use std::collections::HashMap;

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
pub(super) struct Window {
    pub(super) from: Option<Date>,
    pub(super) to: Option<Date>,
    /// `None` when the address did not name a period or a closed custom range.
    preset: Option<Preset>,
}

impl Window {
    /// Reads the address. A named preset keeps the dates the address carries
    /// (the day they were applied on); `sync` moves them once the clock is
    /// known, so a reload on the same day keeps its place in the list.
    pub(super) fn parse(query: &HashMap<String, String>) -> Self {
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
    pub(super) fn resolved(&self, clock: &Clock) -> Option<Period> {
        let preset = self.preset.filter(|preset| *preset != Preset::Custom)?;
        Some(Period::parse(
            |key| (key == "period").then(|| preset.as_str().to_owned()),
            clock.today,
        ))
    }

    /// Takes an applied period. A date field's own limit is an open end.
    pub(super) fn apply(&mut self, period: Period) {
        self.from = (period.from != earliest()).then_some(period.from);
        self.to = (period.to != latest()).then_some(period.to);
        self.preset = (self.from.is_some() && self.to.is_some()).then_some(period.preset);
    }

    pub(super) fn clear(&mut self) {
        *self = Self {
            from: None,
            to: None,
            preset: None,
        };
    }

    /// No date at all: every entry is listed.
    pub(super) fn is_all(&self) -> bool {
        self.from.is_none() && self.to.is_none() && self.preset.is_none()
    }

    /// What the control shows. With every entry listed it holds the shared
    /// default range, which only seeds the custom range dialog. A half-open
    /// window draws its open end as the date field's own limit.
    pub(super) fn widget(&self, clock: &Clock) -> Period {
        if self.is_all() {
            return Period::parse(|_| None, clock.today);
        }
        Period {
            preset: self.preset.unwrap_or(Preset::Custom),
            from: self.from.unwrap_or_else(earliest),
            to: self.to.unwrap_or_else(latest),
        }
    }

    pub(super) fn params(&self) -> HashMap<String, String> {
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
