//! The beta's expiry is enforced by the native command boundary and the UI.

use chrono::Datelike;

/// Shown verbatim when this beta expires.
pub const EXPIRED_MESSAGE: &str = "Thank you for beta testing VectorEffects! The version you are using expired on January 1st, 2027. Please upgrade to use the latest version.";

/// January 1st is the first expired day, in the system's local time zone.
pub fn expired_on(date: chrono::NaiveDate) -> bool {
    date.year() >= 2027
}

/// Read the actual system clock; this has no environment override.
pub fn expired_now() -> bool {
    expired_on(chrono::Local::now().date_naive())
}

/// The only application command available after expiry.
#[tauri::command]
pub fn beta_status() -> Option<String> {
    expired_now().then(|| EXPIRED_MESSAGE.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expires_on_the_first_day_of_2027() {
        for (year, month, day, expired) in [
            (2026, 12, 31, false),
            (2027, 1, 1, true),
            (2028, 2, 29, true),
        ] {
            assert_eq!(
                expired_on(chrono::NaiveDate::from_ymd_opt(year, month, day).unwrap()),
                expired
            );
        }
    }
}
