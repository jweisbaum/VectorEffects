//! Civil time, in UTC, with no dependencies.
//!
//! ERA5 timestamps are "hours since 1959-01-01 00:00:00" on a proleptic
//! Gregorian calendar, and the app never needs a timezone, a locale or a leap
//! second. That is small enough to do exactly, using Howard Hinnant's
//! `days_from_civil` and its inverse, which are correct for any year and shift
//! the calendar's epoch to March so leap days land at the end of a cycle.

use serde::{Deserialize, Serialize};

/// A UTC instant at whole-hour resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Utc {
    /// Four-digit year.
    pub year: i32,
    /// Month, 1-12.
    pub month: u8,
    /// Day, 1-31.
    pub day: u8,
    /// Hour, 0-23.
    pub hour: u8,
}

/// Days from 1970-01-01 to the given civil date, proleptic Gregorian.
///
/// Hinnant's algorithm: shift the year so it starts in March, which puts the
/// leap day last and makes the day-of-era arithmetic branchless.
pub fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let y = i64::from(year) - i64::from(month <= 2);
    let m = i64::from(month);
    let d = i64::from(day);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`].
pub fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = mp + if mp < 10 { 3 } else { -9 }; // [1, 12]
    ((y + i64::from(m <= 2)) as i32, m as u8, d as u8)
}

/// Days in a given month, proleptic Gregorian.
pub fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

impl Utc {
    /// Hours from 1970-01-01T00:00 to this instant.
    pub fn hours_since_unix_epoch(self) -> i64 {
        days_from_civil(self.year, self.month, self.day) * 24 + i64::from(self.hour)
    }

    /// Rebuilds an instant from hours since 1970-01-01T00:00.
    pub fn from_hours_since_unix_epoch(hours: i64) -> Self {
        // Floor division, so instants before 1970 land on the right day.
        let days = hours.div_euclid(24);
        let hour = hours.rem_euclid(24) as u8;
        let (year, month, day) = civil_from_days(days);
        Self {
            year,
            month,
            day,
            hour,
        }
    }

    /// Whether the fields describe a real date and hour.
    pub fn is_valid(self) -> bool {
        (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month)
            && self.hour < 24
    }

    /// Parses `YYYY-MM-DDTHH:MM`, the format an `<input type="datetime-local">`
    /// produces. Any seconds field is accepted and ignored.
    ///
    /// The value is read as UTC. The UI says so, because reading it as local
    /// time would silently shift every exported field.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (date, time) = text.split_once(['T', ' '])?;
        let mut d = date.split('-');
        let year: i32 = d.next()?.parse().ok()?;
        let month: u8 = d.next()?.parse().ok()?;
        let day: u8 = d.next()?.parse().ok()?;
        if d.next().is_some() {
            return None;
        }
        let mut t = time.split(':');
        let hour: u8 = t.next()?.parse().ok()?;
        // Minutes and seconds are accepted so the browser's value parses, but
        // ERA5 is hourly, so anything below the hour would be a lie.
        let minute: u8 = t.next().unwrap_or("0").parse().ok()?;
        if minute != 0 {
            return None;
        }
        let out = Self {
            year,
            month,
            day,
            hour,
        };
        out.is_valid().then_some(out)
    }

    /// Formats as `YYYY-MM-DDTHH:00Z`.
    pub fn to_iso(self) -> String {
        format!(
            "{:04}-{:02}-{:02}T{:02}:00Z",
            self.year, self.month, self.day, self.hour
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unix_epoch_is_day_zero() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    /// The ERA5 epoch. If this drifts, every exported valid time drifts with
    /// it, and nothing else in the pipeline would notice.
    #[test]
    fn the_era5_epoch_is_where_it_should_be() {
        assert_eq!(days_from_civil(1959, 1, 1), -4018);
        assert_eq!(civil_from_days(-4018), (1959, 1, 1));
        let epoch = Utc {
            year: 1959,
            month: 1,
            day: 1,
            hour: 0,
        };
        assert_eq!(epoch.hours_since_unix_epoch(), -4018 * 24);
    }

    #[test]
    fn known_dates_round_trip() {
        for (y, m, d) in [
            (1, 1, 1),
            (1582, 10, 15),
            (1900, 3, 1),
            (1959, 1, 1),
            (1970, 1, 1),
            (2000, 2, 29),
            (2020, 1, 15),
            (2023, 1, 10),
            (2400, 2, 29),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "{y}-{m}-{d}");
        }
    }

    /// 1900 is not a leap year but 2000 is; a naive `% 4` gets one of them
    /// wrong and shifts every subsequent date by a day.
    #[test]
    fn the_gregorian_century_rule_holds() {
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2020, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
        assert_eq!(
            days_from_civil(1900, 3, 1) - days_from_civil(1900, 2, 28),
            1
        );
        assert_eq!(
            days_from_civil(2000, 3, 1) - days_from_civil(2000, 2, 28),
            2
        );
    }

    #[test]
    fn every_hour_over_a_long_span_round_trips() {
        // Ten years of hours across a leap year, stepping by a prime so the
        // walk does not stay in phase with the day.
        let start = Utc {
            year: 1996,
            month: 6,
            day: 3,
            hour: 7,
        }
        .hours_since_unix_epoch();
        let mut h = start;
        while h < start + 24 * 365 * 10 {
            let t = Utc::from_hours_since_unix_epoch(h);
            assert!(t.is_valid(), "{t:?} at hour {h}");
            assert_eq!(t.hours_since_unix_epoch(), h, "{t:?}");
            h += 7;
        }
    }

    /// Instants before 1970 have a negative hour count, where truncating
    /// division would round toward zero and land on the wrong day.
    #[test]
    fn hours_before_the_unix_epoch_floor_correctly() {
        let t = Utc {
            year: 1959,
            month: 1,
            day: 1,
            hour: 1,
        };
        assert_eq!(
            Utc::from_hours_since_unix_epoch(t.hours_since_unix_epoch()),
            t
        );
        let midnight = Utc {
            year: 1969,
            month: 12,
            day: 31,
            hour: 0,
        };
        assert_eq!(midnight.hours_since_unix_epoch(), -24);
        assert_eq!(Utc::from_hours_since_unix_epoch(-24), midnight);
        assert_eq!(
            Utc::from_hours_since_unix_epoch(-1),
            Utc {
                year: 1969,
                month: 12,
                day: 31,
                hour: 23
            }
        );
    }

    #[test]
    fn the_browser_datetime_format_parses() {
        assert_eq!(
            Utc::parse("2020-01-15T12:00"),
            Some(Utc {
                year: 2020,
                month: 1,
                day: 15,
                hour: 12
            })
        );
        assert_eq!(
            Utc::parse("2020-01-15T12:00:00"),
            Some(Utc {
                year: 2020,
                month: 1,
                day: 15,
                hour: 12
            })
        );
        assert_eq!(
            Utc::parse("2020-01-15 12:00"),
            Some(Utc {
                year: 2020,
                month: 1,
                day: 15,
                hour: 12
            })
        );
    }

    /// ERA5 is hourly. A sub-hour request must be refused rather than
    /// rounded, or the file would claim a time it does not hold.
    #[test]
    fn a_time_below_the_hour_is_refused() {
        assert_eq!(Utc::parse("2020-01-15T12:30"), None);
    }

    #[test]
    fn impossible_dates_are_refused() {
        assert_eq!(
            Utc::parse("2023-02-29T00:00"),
            None,
            "2023 is not a leap year"
        );
        assert_eq!(Utc::parse("2020-13-01T00:00"), None);
        assert_eq!(Utc::parse("2020-04-31T00:00"), None);
        assert_eq!(Utc::parse("2020-01-15T24:00"), None);
        assert_eq!(Utc::parse("not a date"), None);
        assert_eq!(Utc::parse("2020-01-15"), None);
        assert!(
            Utc::parse("2020-02-29T00:00").is_some(),
            "2020 is a leap year"
        );
    }

    #[test]
    fn iso_formatting_is_zero_padded_and_marked_utc() {
        let t = Utc {
            year: 999,
            month: 4,
            day: 5,
            hour: 6,
        };
        assert_eq!(t.to_iso(), "0999-04-05T06:00Z");
    }
}
