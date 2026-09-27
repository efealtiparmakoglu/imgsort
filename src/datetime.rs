//! Civil date handling without dependencies: seconds since the Unix epoch
//! to (year, month, day, hour, minute, second) and back, using the
//! standard days-from-civil algorithm (Howard Hinnant's formulation).

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DateTime {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl DateTime {
    #[allow(dead_code)] // part of the calendar API; tests pin it
    /// Days since 1970-01-01 for a proleptic Gregorian date.
    pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
        let y = if m <= 2 { y - 1 } else { y } as i64;
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400; // [0, 399]
        let m = m as i64;
        let d = d as i64;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Inverse of `days_from_civil`.
    pub fn civil_from_days(days: i64) -> (i32, u32, u32) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = mp + if mp < 10 { 3 } else { -9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y as i32, m as u32, d as u32)
    }

    /// Split Unix seconds into a civil date-time (UTC).
    pub fn from_unix(seconds: i64) -> DateTime {
        let days = seconds.div_euclid(86_400);
        let rem = seconds.rem_euclid(86_400);
        let (year, month, day) = DateTime::civil_from_days(days);
        DateTime {
            year,
            month,
            day,
            hour: (rem / 3600) as u32,
            minute: ((rem % 3600) / 60) as u32,
            second: (rem % 60) as u32,
        }
    }

    #[allow(dead_code)] // part of the calendar API; tests pin it
    pub fn to_unix(self) -> i64 {
        DateTime::days_from_civil(self.year, self.month, self.day) * 86_400
            + self.hour as i64 * 3600
            + self.minute as i64 * 60
            + self.second as i64
    }

    /// Parse EXIF's "YYYY:MM:DD HH:MM:SS" (no timezone; treated as UTC).
    /// A bare "YYYY:MM:DD" is accepted with a midnight time.
    pub fn parse_exif(text: &str) -> Option<DateTime> {
        let text = text.trim();
        let (date_part, time_part) = match text.split_once(' ') {
            Some((date, time)) => (date, time),
            None => (text, ""),
        };
        let mut date_fields = date_part.split(':');
        let year: i32 = date_fields.next()?.parse().ok()?;
        let month: u32 = date_fields.next()?.parse().ok()?;
        let day: u32 = date_fields.next()?.parse().ok()?;
        let (hour, minute, second) = if time_part.is_empty() {
            (0, 0, 0)
        } else {
            let mut fields = time_part.split(':');
            let h: u32 = fields.next()?.parse().ok()?;
            let m: u32 = fields.next()?.parse().ok()?;
            let s: u32 = fields.next().unwrap_or("0").parse().unwrap_or(0);
            (h, m, s)
        };
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        Some(DateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trip() {
        let dt = DateTime::from_unix(0);
        assert_eq!(
            dt,
            DateTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }
        );
        assert_eq!(dt.to_unix(), 0);
    }

    #[test]
    fn known_instants() {
        // 2026-09-27T12:34:56Z
        let dt = DateTime::from_unix(1_790_512_496);
        assert_eq!(dt.year, 2026);
        assert_eq!(dt.month, 9);
        assert_eq!(dt.day, 27);
        assert_eq!(dt.hour, 12);
        assert_eq!(dt.minute, 34);
        assert_eq!(dt.second, 56);
        assert_eq!(dt.to_unix(), 1_790_512_496);
    }

    #[test]
    fn leap_day_handling() {
        let dt = DateTime::from_unix(1_709_164_800); // 2024-02-29T00:00:00Z
        assert_eq!((dt.year, dt.month, dt.day), (2024, 2, 29));
        let before = DateTime {
            year: 2024,
            month: 3,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(before.to_unix() - dt.to_unix(), 24 * 3600);
    }

    #[test]
    fn century_rule() {
        // 1900 is not a leap year, 2000 is
        let feb28_1900 = DateTime::days_from_civil(1900, 2, 28);
        let mar1_1900 = DateTime::days_from_civil(1900, 3, 1);
        assert_eq!(mar1_1900 - feb28_1900, 1);
        let feb28_2000 = DateTime::days_from_civil(2000, 2, 28);
        let mar1_2000 = DateTime::days_from_civil(2000, 3, 1);
        assert_eq!(mar1_2000 - feb28_2000, 2);
    }

    #[test]
    fn exif_parsing() {
        let dt = DateTime::parse_exif("2026:09:27 12:34:56").unwrap();
        assert_eq!(dt.year, 2026);
        assert_eq!(dt.month, 9);
        assert_eq!(dt.day, 27);
        assert_eq!(dt.hour, 12);
        let date_only = DateTime::parse_exif("2024:02:29").unwrap();
        assert_eq!(date_only.day, 29);
    }

    #[test]
    fn exif_parsing_rejects_junk() {
        for bad in ["", "not a date", "2026:13:01 00:00:00", "2026:09:32 00:00:00", "2026:xx:01"] {
            assert!(DateTime::parse_exif(bad).is_none(), "{bad} should not parse");
        }
    }

    #[test]
    fn negative_instants() {
        let before_epoch = DateTime::from_unix(-1);
        assert_eq!(before_epoch.year, 1969);
        assert_eq!(before_epoch.month, 12);
        assert_eq!(before_epoch.day, 31);
        assert_eq!(before_epoch.hour, 23);
        assert_eq!(before_epoch.minute, 59);
        assert_eq!(before_epoch.second, 59);
    }
}
