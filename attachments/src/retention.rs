//! The retain-until a source document gets (Bilimbi `Attachments.Retention`):
//! the company's retention years after the end of the financial year its
//! record falls in, at the end of that day in UTC.
use jiff::{
    civil::{Date, DateTime},
    tz::TimeZone,
    Span, Timestamp,
};

/// The years a company keeps source documents until it sets its own; the
/// migration's trigger uses the same number.
pub const DEFAULT_YEARS: i32 = 7;

/// The end of the financial year `date` falls in.
// TODO(finance/ledger periods): the ledger's fiscal year covering the date
// decides this once fiscal years land (L2): the year's end when one covers
// the date, else the company's year end projected in twelve-month steps from
// its nearest opened year. A company with no fiscal year at all takes 31
// December of the date's year, which is all a company can have until then.
pub(crate) fn financial_year_end(date: Date) -> Date {
    Date::constant(date.year(), 12, 31)
}

/// The end of the day `years` after `year_end`, in UTC.
pub(crate) fn until(year_end: Date, years: i32) -> Timestamp {
    let shifted = year_end
        .checked_add(Span::new().years(i64::from(years)))
        .unwrap_or(year_end);
    end_of_day(shifted)
}

/// The last instant of `date` in UTC, at the microsecond Base Artifacts stores.
pub(crate) fn end_of_day(date: Date) -> Timestamp {
    DateTime::from(date)
        .with()
        .hour(23)
        .minute(59)
        .second(59)
        .subsec_nanosecond(999_999_000)
        .build()
        .ok()
        .and_then(|moment| moment.to_zoned(TimeZone::UTC).ok())
        .map_or(Timestamp::MAX, |zoned| zoned.timestamp())
}
