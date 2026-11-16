//! The local time, without a time zone database: Windows gives its own local clock (other
//! systems fall back to UTC).

use std::time::{SystemTime, UNIX_EPOCH};

/// A moment as the clock on the wall shows it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Local {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    /// 0 = Sunday.
    pub weekday: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

const WEEKDAYS: [&str; 7] = [
    "dimanche", "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi",
];

impl Local {
    /// `21:34:07`.
    pub fn time(&self) -> String {
        format!("{:02}:{:02}:{:02}", self.hour, self.minute, self.second)
    }

    /// `21:34`.
    pub fn hour_minute(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// `16/11/2026`.
    pub fn date(&self) -> String {
        format!("{:02}/{:02}/{:04}", self.day, self.month, self.year)
    }

    /// `lundi`.
    pub fn weekday_name(&self) -> &'static str {
        WEEKDAYS[usize::from(self.weekday % 7)]
    }

    /// Milliseconds since local midnight.
    pub fn since_midnight_ms(&self) -> u64 {
        ((u64::from(self.hour) * 60 + u64::from(self.minute)) * 60 + u64::from(self.second)) * 1000
            + u64::from(self.millis)
    }
}

/// Unix time in milliseconds.
pub fn epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

/// The local time now.
pub fn now() -> Local {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            weekday: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            millis: u16,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLocalTime(time: *mut SystemTime);
        }
        let mut now = SystemTime::default();
        // SAFETY: GetLocalTime only fills the structure it is given.
        unsafe { GetLocalTime(&mut now) };
        Local {
            year: now.year,
            month: now.month,
            day: now.day,
            weekday: now.weekday,
            hour: now.hour,
            minute: now.minute,
            second: now.second,
            millis: now.millis,
        }
    }
    #[cfg(not(windows))]
    {
        let ms = epoch_ms();
        let seconds = ms / 1000;
        let days = seconds / 86_400;
        let (year, month, day) = civil(days as i64);
        Local {
            year,
            month,
            day,
            weekday: ((days + 4) % 7) as u16,
            hour: (seconds / 3600 % 24) as u16,
            minute: (seconds / 60 % 60) as u16,
            second: (seconds % 60) as u16,
            millis: (ms % 1000) as u16,
        }
    }
}

/// Days since 1970 to a calendar date (Howard Hinnant's algorithm).
#[cfg(not(windows))]
fn civil(days: i64) -> (u16, u16, u16) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year as u16, month as u16, day as u16)
}

/// Today's local midnight, as Unix milliseconds: a timer started there shows the time of day.
pub fn midnight_ms() -> u64 {
    epoch_ms().saturating_sub(now().since_midnight_ms())
}

/// The local time as `HH:MM:SS`.
pub fn clock() -> String {
    now().time()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_times_read_like_a_french_clock() {
        let moment = Local {
            year: 2026,
            month: 11,
            day: 6,
            weekday: 1,
            hour: 9,
            minute: 5,
            second: 3,
            millis: 250,
        };
        assert_eq!(moment.time(), "09:05:03");
        assert_eq!(moment.hour_minute(), "09:05");
        assert_eq!(moment.date(), "06/11/2026");
        assert_eq!(moment.weekday_name(), "lundi");
        assert_eq!(moment.since_midnight_ms(), (9 * 3600 + 5 * 60 + 3) * 1000 + 250);
        assert!(midnight_ms() <= epoch_ms());
        assert!(epoch_ms() - midnight_ms() < 86_400_000);
    }
}
