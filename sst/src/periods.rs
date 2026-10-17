//! Bilimbi.Finance.Sst.Periods: taxable periods and return due dates under a
//! registration's cycle.
//!
//! The first period of a cycle runs from its `period_start` to the end of the
//! month `period_months` months on, counting the start's own month, as section
//! 25(1) of the Sales Tax Act 2018 and of the Service Tax Act 2018 set the
//! first period: from the date a person is registered to the end of the month
//! after it, for the usual two months. Each later period is `period_months`
//! whole calendar months.
//!
//! A return is due on the last day of the month after its period ends
//! (section 26(1) of both Acts) on the usual two-month cycle, and 30 days
//! after the period ends on a cycle the Director General varied (section
//! 26(2)). A due date that falls on a weekend or public holiday moves to the
//! next working day in practice; holidays are not this module's data, so the
//! date shown is the statutory one.
use jiff::civil::Date;
use jiff::ToSpan;

/// A registration's cycle: its first period's first day and its length.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cycle {
    pub first: Date,
    pub months: i32,
}

/// The last day of the month `months_on` months after `date`'s.
fn month_end(date: Date, months_on: i32) -> Option<Date> {
    date.first_of_month()
        .checked_add(months_on.months())
        .ok()
        .map(|d| d.last_of_month())
}

fn months_between(from: Date, to: Date) -> i32 {
    (i32::from(to.year()) - i32::from(from.year())) * 12 + i32::from(to.month())
        - i32::from(from.month())
}

impl Cycle {
    fn first_end(self) -> Option<Date> {
        month_end(self.first, self.months - 1)
    }

    /// The period that starts on `start`, when one does.
    pub fn period(self, start: Date) -> Option<(Date, Date)> {
        let first_end = self.first_end()?;
        if start == self.first {
            Some((self.first, first_end))
        } else if start > first_end
            && start.day() == 1
            && (months_between(self.first, start) - self.months) % self.months == 0
        {
            Some((start, month_end(start, self.months - 1)?))
        } else {
            None
        }
    }

    /// The period that contains `date`, unless `date` precedes the cycle.
    pub fn containing(self, date: Date) -> Option<(Date, Date)> {
        let first_end = self.first_end()?;
        if date < self.first {
            None
        } else if date <= first_end {
            Some((self.first, first_end))
        } else {
            let offset = (months_between(self.first, date) - self.months) / self.months
                * self.months
                + self.months;
            let start = self.first.first_of_month().checked_add(offset.months()).ok()?;
            self.period(start)
        }
    }

    /// Up to `count` periods, newest first, ending with the one that contains
    /// `date` (or the cycle's first, when `date` precedes it).
    pub fn recent(self, date: Date, count: usize) -> Vec<(Date, Date)> {
        let mut current = self
            .containing(date)
            .or_else(|| self.period(self.first));
        let mut periods = Vec::new();
        while let Some((start, finish)) = current {
            if periods.len() == count {
                break;
            }
            periods.push((start, finish));
            current = if start == self.first {
                None
            } else {
                start.yesterday().ok().and_then(|d| self.containing(d))
            };
        }
        periods
    }

    /// The statutory due date of a return for a period ending on `period_end`.
    pub fn due_date(self, period_end: Date) -> Option<Date> {
        if self.months == 2 {
            period_end.tomorrow().ok().map(|d| d.last_of_month())
        } else {
            period_end.checked_add(30.days()).ok()
        }
    }
}
