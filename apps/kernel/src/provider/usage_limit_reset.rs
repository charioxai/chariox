//! When the provider says a usage limit lifts: the time in its own words, such as
//! `resets 4pm (UTC)`, `resets Oct 28, 9pm (UTC)` or `resets 3:50am (Europe/Madrid)`.

use chrono::{Datelike, TimeZone};

/// The instant the limit named in `text` lifts, or `None` when the text names none the kernel
/// can read. A time without a date is the next such time after `now_ms`; a time without a
/// zone is ambiguous: callers use the account meter's absolute reset time instead.
pub(crate) fn usage_limit_reset_at_ms(text: &str, now_ms: u64) -> Option<u64> {
    let lowered = text.to_ascii_lowercase();
    let start = lowered.find("resets ")? + "resets ".len();
    let words = text[start..].split_whitespace().collect::<Vec<_>>();
    let (date, rest) = match words.as_slice() {
        [month, day, rest @ ..] if month_number(month).is_some() => {
            let day = day.trim_end_matches(',').parse::<u32>().ok()?;
            (Some((month_number(month)?, day)), rest)
        }
        rest => (None, rest),
    };
    let (hour, minute) = clock_time(rest.first()?)?;
    let zone = rest
        .get(1)
        .map(|zone| zone.trim_matches(|c| c == '(' || c == ')' || c == '.'));
    match zone {
        Some(zone) if !zone.is_empty() => {
            let zone = zone.parse::<chrono_tz::Tz>().ok()?;
            instant_in(&zone, date, hour, minute, now_ms)
        }
        _ => None,
    }
}

fn month_number(word: &str) -> Option<u32> {
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let word = word.to_ascii_lowercase();
    months
        .iter()
        .position(|month| word.starts_with(month))
        .map(|index| index as u32 + 1)
}

/// `4pm`, `3:50am`, `12am`.
fn clock_time(word: &str) -> Option<(u32, u32)> {
    let word = word.to_ascii_lowercase();
    let (digits, pm) = if let Some(digits) = word.strip_suffix("pm") {
        (digits, true)
    } else {
        (word.strip_suffix("am")?, false)
    };
    let (hour, minute) = match digits.split_once(':') {
        Some((hour, minute)) => (hour.parse::<u32>().ok()?, minute.parse::<u32>().ok()?),
        None => (digits.parse::<u32>().ok()?, 0),
    };
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    Some(((hour % 12) + if pm { 12 } else { 0 }, minute))
}

fn instant_in<Tz: TimeZone>(
    zone: &Tz,
    date: Option<(u32, u32)>,
    hour: u32,
    minute: u32,
    now_ms: u64,
) -> Option<u64> {
    let now = zone
        .timestamp_millis_opt(i64::try_from(now_ms).ok()?)
        .single()?;
    let at = |year: i32, month: u32, day: u32| {
        zone.with_ymd_and_hms(year, month, day, hour, minute, 0)
            .earliest()
            .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
    };
    match date {
        Some((month, day)) => {
            let this_year = at(now.year(), month, day)?;
            // A date months in the past is next year's.
            if this_year + 24 * 3_600_000 < now_ms {
                at(now.year() + 1, month, day)
            } else {
                Some(this_year)
            }
        }
        None => {
            let today = at(now.year(), now.month(), now.day())?;
            if today > now_ms {
                Some(today)
            } else {
                let tomorrow = now.date_naive().succ_opt()?;
                at(tomorrow.year(), tomorrow.month(), tomorrow.day())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-11-15 14:59:00 UTC, when the weekly limit stopped two lanes.
    const NOW: u64 = 1_794_754_740_000;

    fn utc(text: &str) -> Option<u64> {
        usage_limit_reset_at_ms(text, NOW)
    }

    #[test]
    fn a_time_with_a_zone_is_the_next_such_time() {
        // 16:00 UTC the same day.
        assert_eq!(
            utc("You've hit your weekly limit \u{b7} resets 4pm (UTC)"),
            Some(1_794_758_400_000)
        );
        // 02:00 UTC is tomorrow.
        assert_eq!(
            utc("You've hit your session limit \u{b7} resets 2am (UTC)"),
            Some(1_794_794_400_000)
        );
    }

    #[test]
    fn a_date_names_the_day() {
        // Oct 28 is nearly a year back: next year's.
        let at = utc("You've hit your weekly limit \u{b7} resets Oct 28, 9pm (UTC)").unwrap();
        assert!(at > NOW + 300 * 24 * 3_600_000);
        let near = utc("limit \u{b7} resets Nov 20, 9pm (UTC)").unwrap();
        assert_eq!(near, 1_795_208_400_000);
    }

    #[test]
    fn a_named_zone_and_minutes_are_read() {
        // 3:50am Madrid (UTC+1) is 02:50 UTC.
        assert_eq!(
            utc("limit \u{b7} resets 3:50am (Europe/Madrid)"),
            Some(1_794_797_400_000)
        );
    }

    #[test]
    fn a_text_without_a_readable_time_names_none() {
        assert_eq!(utc("You've hit your usage limit"), None);
        assert_eq!(utc("resets soon (UTC)"), None);
        assert_eq!(utc("resets 4pm (Not/AZone)"), None);
        assert_eq!(utc("resets 13pm (UTC)"), None);
        assert_eq!(utc("resets 4pm"), None);
        assert_eq!(utc("try again at 3:06 PM"), None);
    }
}
