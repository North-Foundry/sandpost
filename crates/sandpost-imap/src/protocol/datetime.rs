//! IMAP dates: SEARCH `date` arguments, INTERNALDATE formatting, and civil-date arithmetic.
//!
//! Internal dates are Unix seconds. Sandpost records them in UTC, so INTERNALDATE is written with
//! a `+0000` zone and date-only SEARCH criteria compare UTC calendar days.

/// English month abbreviations used by IMAP `date-month`.
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A calendar day without time or zone, as used by date-only SEARCH keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CivilDate {
    pub year: i64,
    pub month: u32,
    pub day: u32,
}

impl CivilDate {
    /// Parse `date-text` (`d-Mon-yyyy` or `dd-Mon-yyyy`), rejecting impossible days.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('-');
        let (day, month, year) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some()
            || !(1..=2).contains(&day.len())
            || year.len() != 4
            || !day
                .bytes()
                .chain(year.bytes())
                .all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        let month = MONTHS
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(month))?
            as u32
            + 1;
        let date = Self {
            year: year.parse().ok()?,
            month,
            day: day.parse().ok()?,
        };
        (date.day >= 1 && date.day <= days_in_month(date.year, date.month)).then_some(date)
    }

    /// The UTC calendar day containing a Unix timestamp.
    pub fn from_unix_seconds(seconds: i64) -> Self {
        let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
        Self { year, month, day }
    }
}

/// Format Unix seconds as an IMAP `date-time`: `"dd-Mon-yyyy hh:mm:ss +0000"` with a
/// space-padded day (RFC 3501 section 9 `date-day-fixed`).
pub fn format_internal_date(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "\"{day:>2}-{}-{year:04} {:02}:{:02}:{:02} +0000\"",
        MONTHS[(month - 1) as usize],
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// Parse an APPEND `date-time` into Unix seconds, honoring its numeric zone.
pub fn parse_internal_date(text: &str) -> Option<i64> {
    let (date, rest) = text.trim_start().split_once(' ')?;
    let (time, zone) = rest.split_once(' ')?;
    let date = CivilDate::parse(date)?;
    let mut fields = time.split(':');
    let (hour, minute, second) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some()
        || [hour, minute, second]
            .iter()
            .any(|field| field.len() != 2 || !field.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let (hour, minute, second): (i64, i64, i64) = (
        hour.parse().ok()?,
        minute.parse().ok()?,
        second.parse().ok()?,
    );
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let zone = zone.as_bytes();
    if zone.len() != 5
        || !matches!(zone[0], b'+' | b'-')
        || !zone[1..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let offset_hours = i64::from(zone[1] - b'0') * 10 + i64::from(zone[2] - b'0');
    let offset_minutes = i64::from(zone[3] - b'0') * 10 + i64::from(zone[4] - b'0');
    let offset = (offset_hours * 3600 + offset_minutes * 60) * if zone[0] == b'-' { -1 } else { 1 };
    Some(
        days_from_civil(date.year, date.month, date.day) * 86_400
            + hour * 3600
            + minute * 60
            + second
            - offset,
    )
}

/// Number of days in a month of the proleptic Gregorian calendar.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => 0,
    }
}

/// Convert days since 1970-01-01 to a proleptic Gregorian (year, month, day).
///
/// Howard Hinnant's `civil_from_days` algorithm, exact for the whole `i64` day range in use.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Convert a proleptic Gregorian date to days since 1970-01-01 (Hinnant's `days_from_civil`).
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INTERNALDATE uses a space-padded day, English month, and UTC zone.
    #[test]
    fn internal_dates_use_fixed_format() {
        assert_eq!(format_internal_date(0), "\" 1-Jan-1970 00:00:00 +0000\"");
        assert_eq!(
            format_internal_date(1_700_000_000),
            "\"14-Nov-2023 22:13:20 +0000\""
        );
        assert_eq!(
            format_internal_date(951_825_599),
            "\"29-Feb-2000 11:59:59 +0000\""
        );
    }

    /// SEARCH dates accept one- or two-digit days, any month case, and reject invalid days.
    #[test]
    fn search_dates_parse_and_validate() {
        assert_eq!(
            CivilDate::parse("1-feb-1994"),
            Some(CivilDate {
                year: 1994,
                month: 2,
                day: 1
            })
        );
        assert!(CivilDate::parse("29-Feb-2024").is_some());
        assert!(CivilDate::parse("29-Feb-2023").is_none());
        assert!(CivilDate::parse("31-Apr-2024").is_none());
        assert!(CivilDate::parse("001-Jan-2024").is_none());
        assert!(CivilDate::parse("1-January-2024").is_none());
        assert!(CivilDate::parse("1-Jan-24").is_none());
    }

    /// Civil-date conversion round-trips across leap years and the epoch.
    #[test]
    fn civil_dates_round_trip() {
        for days in [-719_468, -1, 0, 59, 10_956, 19_723, 2_932_896] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(
            CivilDate::from_unix_seconds(-1),
            CivilDate {
                year: 1969,
                month: 12,
                day: 31
            }
        );
    }

    /// APPEND date-times honor their numeric zone.
    #[test]
    fn append_dates_apply_zone_offsets() {
        assert_eq!(parse_internal_date(" 1-Jan-1970 01:00:00 +0100"), Some(0));
        assert_eq!(
            parse_internal_date("01-Jan-1970 00:00:00 -0030"),
            Some(1800)
        );
        assert!(parse_internal_date("01-Jan-1970 24:00:00 +0000").is_none());
        assert!(parse_internal_date("01-Jan-1970 00:00:00 UTC").is_none());
    }
}
