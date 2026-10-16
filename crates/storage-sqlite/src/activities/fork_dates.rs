//! Fork: date-only activity dates are stored at noon UTC (upstream: midnight).

use chrono::{DateTime, NaiveDate, TimeZone, Utc};

/// Instant stored for a date-only activity date.
///
/// Noon UTC keeps the calendar day for every timezone from UTC-11 to UTC+11.
/// Midnight UTC (the previous behavior) is already the previous evening in
/// the Americas, which shifted date-only imports one day earlier there.
pub(crate) fn date_only_activity_instant(date: NaiveDate) -> DateTime<Utc> {
    Utc.from_utc_datetime(&date.and_hms_opt(12, 0, 0).expect("noon is always valid"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn date_only_activity_dates_keep_their_calendar_day_across_timezones() {
        let date = NaiveDate::from_ymd_opt(2025, 2, 12).unwrap();
        let instant = date_only_activity_instant(date);
        assert_eq!(instant.to_rfc3339(), "2025-02-12T12:00:00+00:00");

        // Vancouver standard/daylight time and a far-east offset all see the
        // same day; midnight UTC used to land on 2025-02-11 in the Americas.
        for hours in [-8, -7, 11] {
            let offset = FixedOffset::east_opt(hours * 3600).unwrap();
            assert_eq!(
                instant.with_timezone(&offset).date_naive(),
                date,
                "UTC{hours:+}"
            );
        }
    }
}
