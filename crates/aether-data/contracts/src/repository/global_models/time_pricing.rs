//! Parsing and evaluation of the `time_pricing` block inside `tiered_pricing`.
//!
//! Billing follows the clock, not the calendar of whoever happens to be running the gateway:
//! the configuration owns an explicit IANA timezone and every window is evaluated against the
//! moment the final upstream request was dispatched. A request that starts before a window ends
//! is billed by its start instant even when the response outlives the window.
//!
//! Windows are half-open (`[start, end)`), so `09:00-12:00` bills 09:00 as peak and 12:00 as
//! off-peak. A timestamp that matches no window is billed at the implicit `1.0` multiplier; the
//! configuration only ever describes the exceptions.

use chrono::{Datelike, TimeZone, Timelike, Utc, Weekday};
use chrono_tz::Tz;
use serde_json::Value;

pub const TIME_PRICING_FIELD: &str = "time_pricing";
pub const TIMEZONE_FIELD: &str = "timezone";
pub const WINDOWS_FIELD: &str = "windows";
pub const MAX_TIME_PRICING_MULTIPLIER: f64 = 100.0;

const MINUTES_PER_DAY: u16 = 24 * 60;
const WEEKDAY_NAMES: [&str; 7] = [
    "monday",
    "tuesday",
    "wednesday",
    "thursday",
    "friday",
    "saturday",
    "sunday",
];

#[derive(Debug, Clone, PartialEq)]
pub struct TimePricingWindow {
    pub id: String,
    /// Indexed by `Weekday::num_days_from_monday`, so index 0 is Monday.
    pub weekdays: [bool; 7],
    pub start_minute_of_day: u16,
    pub end_minute_of_day: u16,
    pub price_multiplier: f64,
}

impl TimePricingWindow {
    pub fn covers(&self, weekday: Weekday, minute_of_day: u16) -> bool {
        self.weekdays[weekday.num_days_from_monday() as usize]
            && self.start_minute_of_day <= minute_of_day
            && minute_of_day < self.end_minute_of_day
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TimePricingConfig {
    pub timezone: String,
    pub windows: Vec<TimePricingWindow>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TimePricingCatalogState {
    Inherit,
    Disabled,
    Configured(TimePricingConfig),
}

/// Parses the `time_pricing` block of a `tiered_pricing` catalog.
///
/// `Ok(None)` means the catalog does not opt into time based pricing at all. A present but
/// malformed block is an error: silently ignoring it would bill every hour at the standard price
/// while the operator believes peak pricing is active.
pub fn parse_time_pricing(
    field_name: &str,
    tiered_pricing: Option<&Value>,
) -> Result<Option<TimePricingConfig>, String> {
    Ok(
        match parse_time_pricing_catalog_state(field_name, tiered_pricing)? {
            TimePricingCatalogState::Configured(config) => Some(config),
            TimePricingCatalogState::Inherit | TimePricingCatalogState::Disabled => None,
        },
    )
}

/// Preserves the distinction needed by Provider overrides: an absent key inherits the global
/// configuration, while an explicit `null` disables global time pricing for that Provider model.
pub fn parse_time_pricing_catalog_state(
    field_name: &str,
    tiered_pricing: Option<&Value>,
) -> Result<TimePricingCatalogState, String> {
    let Some(tiered_pricing) = tiered_pricing else {
        return Ok(TimePricingCatalogState::Inherit);
    };
    let Some(raw) = tiered_pricing.get(TIME_PRICING_FIELD) else {
        return Ok(TimePricingCatalogState::Inherit);
    };
    if raw.is_null() {
        return Ok(TimePricingCatalogState::Disabled);
    }

    let field = format!("{field_name}.{TIME_PRICING_FIELD}");
    let Some(object) = raw.as_object() else {
        return Err(format!("{field} must be an object"));
    };

    let timezone = object
        .get(TIMEZONE_FIELD)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{field}.{TIMEZONE_FIELD} must be a non-empty IANA timezone"))?;
    if timezone.parse::<Tz>().is_err() {
        return Err(format!(
            "{field}.{TIMEZONE_FIELD} `{timezone}` is not a known IANA timezone"
        ));
    }

    let windows_value = object
        .get(WINDOWS_FIELD)
        .ok_or_else(|| format!("{field}.{WINDOWS_FIELD} must be an array"))?;
    let Some(windows_array) = windows_value.as_array() else {
        return Err(format!("{field}.{WINDOWS_FIELD} must be an array"));
    };
    if windows_array.is_empty() {
        return Err(format!(
            "{field}.{WINDOWS_FIELD} must contain at least one window"
        ));
    }

    let mut windows = Vec::with_capacity(windows_array.len());
    for (index, window) in windows_array.iter().enumerate() {
        windows.push(parse_window(&field, index, window)?);
    }
    reject_overlapping_windows(&field, &windows)?;

    Ok(TimePricingCatalogState::Configured(TimePricingConfig {
        timezone: timezone.to_string(),
        windows,
    }))
}

/// Returns the window covering `at_unix_ms` and its multiplier, or `None` for off-peak.
///
/// `None` is also the answer for a timestamp that cannot be represented, which keeps an
/// out-of-range clock from turning into a silent surcharge.
pub fn resolve_time_pricing_window(
    config: &TimePricingConfig,
    at_unix_ms: i64,
) -> Option<&TimePricingWindow> {
    let timezone: Tz = config.timezone.parse().ok()?;
    let instant = Utc.timestamp_millis_opt(at_unix_ms).single()?;
    let local = instant.with_timezone(&timezone);
    let minute_of_day = u16::try_from(local.hour() * 60 + local.minute()).ok()?;
    config
        .windows
        .iter()
        .find(|window| window.covers(local.weekday(), minute_of_day))
}

fn parse_window(field: &str, index: usize, value: &Value) -> Result<TimePricingWindow, String> {
    let scope = format!("{field}.{WINDOWS_FIELD}[{index}]");
    let Some(object) = value.as_object() else {
        return Err(format!("{scope} must be an object"));
    };

    let id = object
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{scope}.id must be a non-empty string"))?
        .to_string();

    let weekdays = parse_weekdays(&format!("{scope}.weekdays"), object.get("weekdays"))?;
    let start_minute_of_day = parse_time_of_day(&format!("{scope}.start"), object.get("start"))?;
    let end_minute_of_day = parse_time_of_day(&format!("{scope}.end"), object.get("end"))?;
    if end_minute_of_day <= start_minute_of_day {
        return Err(format!(
            "{scope} must satisfy start < end; windows cannot wrap across midnight"
        ));
    }

    let Some(multiplier) = object.get("price_multiplier").and_then(Value::as_f64) else {
        return Err(format!(
            "{scope}.price_multiplier must be a non-negative finite number"
        ));
    };
    if !multiplier.is_finite() || !(0.0..=MAX_TIME_PRICING_MULTIPLIER).contains(&multiplier) {
        return Err(format!(
            "{scope}.price_multiplier must be between 0 and {MAX_TIME_PRICING_MULTIPLIER}"
        ));
    }

    Ok(TimePricingWindow {
        id,
        weekdays,
        start_minute_of_day,
        end_minute_of_day,
        price_multiplier: multiplier,
    })
}

fn parse_weekdays(field: &str, value: Option<&Value>) -> Result<[bool; 7], String> {
    let Some(value) = value else {
        return Err(format!(
            "{field} must be a non-empty array of weekday names"
        ));
    };
    let Some(entries) = value.as_array() else {
        return Err(format!(
            "{field} must be a non-empty array of weekday names"
        ));
    };
    if entries.is_empty() {
        return Err(format!(
            "{field} must be a non-empty array of weekday names"
        ));
    }

    let mut weekdays = [false; 7];
    for entry in entries {
        let Some(name) = entry.as_str() else {
            return Err(format!("{field} must contain weekday names only"));
        };
        let index = weekday_index(name)
            .ok_or_else(|| format!("{field} contains unknown weekday `{}`", name.trim()))?;
        weekdays[index] = true;
    }
    Ok(weekdays)
}

fn weekday_index(name: &str) -> Option<usize> {
    let name = name.trim().to_ascii_lowercase();
    if name.is_empty() {
        return None;
    }
    // Accept the full name or its three letter abbreviation only: `t` and `th` would otherwise
    // resolve by array order rather than by intent.
    WEEKDAY_NAMES
        .iter()
        .position(|candidate| *candidate == name || candidate[..3] == name)
}

fn parse_time_of_day(field: &str, value: Option<&Value>) -> Result<u16, String> {
    let Some(raw) = value.and_then(Value::as_str) else {
        return Err(format!("{field} must be a `HH:MM` time of day"));
    };
    let raw = raw.trim();
    let Some((hour, minute)) = raw.split_once(':') else {
        return Err(format!("{field} must be a `HH:MM` time of day"));
    };
    if hour.len() != 2 || minute.len() != 2 {
        return Err(format!("{field} must be a `HH:MM` time of day"));
    }
    if !hour.bytes().all(|byte| byte.is_ascii_digit())
        || !minute.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!("{field} must be a `HH:MM` time of day"));
    }
    let hour: u16 = hour
        .parse()
        .map_err(|_| format!("{field} must be a `HH:MM` time of day"))?;
    let minute: u16 = minute
        .parse()
        .map_err(|_| format!("{field} must be a `HH:MM` time of day"))?;
    if hour > 24 || minute > 59 {
        return Err(format!("{field} must be a `HH:MM` time of day"));
    }
    let minute_of_day = hour * 60 + minute;
    // `24:00` is the only way to express the end of the day; `24:30` is nonsense.
    if minute_of_day > MINUTES_PER_DAY || (minute_of_day == MINUTES_PER_DAY && minute != 0) {
        return Err(format!("{field} must be a `HH:MM` time of day"));
    }
    Ok(minute_of_day)
}

fn reject_overlapping_windows(field: &str, windows: &[TimePricingWindow]) -> Result<(), String> {
    let mut ids = std::collections::BTreeSet::new();
    for window in windows {
        if !ids.insert(window.id.as_str()) {
            return Err(format!(
                "{field}.{WINDOWS_FIELD} contains duplicate window id `{}`",
                window.id
            ));
        }
    }

    for (index, left) in windows.iter().enumerate() {
        for right in windows.iter().skip(index + 1) {
            let shares_weekday = left
                .weekdays
                .iter()
                .zip(right.weekdays.iter())
                .any(|(left, right)| *left && *right);
            if !shares_weekday {
                continue;
            }
            let overlaps = left.start_minute_of_day < right.end_minute_of_day
                && right.start_minute_of_day < left.end_minute_of_day;
            if overlaps {
                return Err(format!(
                    "{field}.{WINDOWS_FIELD} windows `{}` and `{}` overlap",
                    left.id, right.id
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use serde_json::json;

    fn workday_config() -> TimePricingConfig {
        parse_time_pricing(
            "models.tiered_pricing",
            Some(&json!({
                "time_pricing": {
                    "timezone": "Asia/Shanghai",
                    "windows": [
                        {
                            "id": "workday-morning-peak",
                            "weekdays": ["monday", "tuesday", "wednesday", "thursday", "friday"],
                            "start": "09:00",
                            "end": "12:00",
                            "price_multiplier": 2
                        },
                        {
                            "id": "workday-afternoon-peak",
                            "weekdays": ["monday", "tuesday", "wednesday", "thursday", "friday"],
                            "start": "14:00",
                            "end": "18:00",
                            "price_multiplier": 2
                        }
                    ]
                }
            })),
        )
        .expect("config should parse")
        .expect("config should be present")
    }

    fn shanghai_millis(date: &str, time: &str) -> i64 {
        let date = NaiveDate::parse_from_str(date, "%Y-%m-%d").expect("naive date");
        let time = chrono::NaiveTime::parse_from_str(time, "%H:%M").expect("naive time");
        let naive = date.and_time(time);
        chrono_tz::Asia::Shanghai
            .from_local_datetime(&naive)
            .single()
            .expect("unambiguous local time")
            .timestamp_millis()
    }

    #[test]
    fn resolves_boundary_minutes_as_half_open_windows() {
        let config = workday_config();
        // 2026-09-21 is a Monday.
        for (time, expected) in [
            ("08:59", None),
            ("09:00", Some("workday-morning-peak")),
            ("11:59", Some("workday-morning-peak")),
            ("12:00", None),
            ("13:59", None),
            ("14:00", Some("workday-afternoon-peak")),
            ("17:59", Some("workday-afternoon-peak")),
            ("18:00", None),
        ] {
            let resolved =
                resolve_time_pricing_window(&config, shanghai_millis("2026-09-21", time));
            assert_eq!(
                resolved.map(|window| window.id.as_str()),
                expected,
                "unexpected window for {time}"
            );
        }
    }

    #[test]
    fn weekend_is_never_peak() {
        let config = workday_config();
        // 2026-09-20 is a Sunday.
        assert!(
            resolve_time_pricing_window(&config, shanghai_millis("2026-09-20", "10:00")).is_none()
        );
        assert!(
            resolve_time_pricing_window(&config, shanghai_millis("2026-09-19", "15:00")).is_none()
        );
    }

    #[test]
    fn configuration_timezone_governs_not_the_utc_hour() {
        let config = workday_config();
        // 2026-09-21 01:30 UTC is 09:30 in Shanghai, so it is peak.
        let utc = chrono::NaiveDate::from_ymd_opt(2026, 9, 21)
            .unwrap()
            .and_hms_opt(1, 30, 0)
            .unwrap();
        let at = Utc.from_utc_datetime(&utc).timestamp_millis();
        assert_eq!(
            resolve_time_pricing_window(&config, at).map(|window| window.id.as_str()),
            Some("workday-morning-peak")
        );
    }

    #[test]
    fn absent_config_is_not_an_error() {
        assert!(parse_time_pricing("models.tiered_pricing", None)
            .expect("absent catalog")
            .is_none());
        assert!(
            parse_time_pricing("models.tiered_pricing", Some(&json!({"tiers": []})))
                .expect("catalog without time pricing")
                .is_none()
        );
    }

    #[test]
    fn catalog_state_distinguishes_inheritance_from_explicit_disable() {
        assert_eq!(
            parse_time_pricing_catalog_state("models.tiered_pricing", Some(&json!({})))
                .expect("missing field should inherit"),
            TimePricingCatalogState::Inherit
        );
        assert_eq!(
            parse_time_pricing_catalog_state(
                "models.tiered_pricing",
                Some(&json!({"time_pricing": null})),
            )
            .expect("null should explicitly disable"),
            TimePricingCatalogState::Disabled
        );
    }

    #[test]
    fn rejects_malformed_configurations() {
        for (label, value) in [
            (
                "unknown timezone",
                json!({"timezone": "Mars/Olympus", "windows": []}),
            ),
            (
                "empty windows",
                json!({"timezone": "Asia/Shanghai", "windows": []}),
            ),
            (
                "bad time",
                json!({"timezone": "Asia/Shanghai", "windows": [{"id": "a", "weekdays": ["monday"], "start": "9:00", "end": "12:00", "price_multiplier": 2}]}),
            ),
            (
                "reversed range",
                json!({"timezone": "Asia/Shanghai", "windows": [{"id": "a", "weekdays": ["monday"], "start": "12:00", "end": "09:00", "price_multiplier": 2}]}),
            ),
            (
                "negative multiplier",
                json!({"timezone": "Asia/Shanghai", "windows": [{"id": "a", "weekdays": ["monday"], "start": "09:00", "end": "12:00", "price_multiplier": -1}]}),
            ),
            (
                "excessive multiplier",
                json!({"timezone": "Asia/Shanghai", "windows": [{"id": "a", "weekdays": ["monday"], "start": "09:00", "end": "12:00", "price_multiplier": 101}]}),
            ),
            (
                "unknown weekday",
                json!({"timezone": "Asia/Shanghai", "windows": [{"id": "a", "weekdays": ["funday"], "start": "09:00", "end": "12:00", "price_multiplier": 2}]}),
            ),
            (
                "empty weekdays",
                json!({"timezone": "Asia/Shanghai", "windows": [{"id": "a", "weekdays": [], "start": "09:00", "end": "12:00", "price_multiplier": 2}]}),
            ),
            (
                "duplicate id",
                json!({"timezone": "Asia/Shanghai", "windows": [
                    {"id": "a", "weekdays": ["monday"], "start": "09:00", "end": "12:00", "price_multiplier": 2},
                    {"id": "a", "weekdays": ["tuesday"], "start": "09:00", "end": "12:00", "price_multiplier": 2}
                ]}),
            ),
            (
                "overlapping windows",
                json!({"timezone": "Asia/Shanghai", "windows": [
                    {"id": "a", "weekdays": ["monday"], "start": "09:00", "end": "12:00", "price_multiplier": 2},
                    {"id": "b", "weekdays": ["monday"], "start": "11:00", "end": "13:00", "price_multiplier": 3}
                ]}),
            ),
        ] {
            let error = parse_time_pricing(
                "models.tiered_pricing",
                Some(&json!({ "time_pricing": value })),
            )
            .unwrap_err();
            assert!(!error.is_empty(), "{label} should be rejected");
        }
    }

    #[test]
    fn adjacent_windows_do_not_count_as_overlapping() {
        parse_time_pricing(
            "models.tiered_pricing",
            Some(&json!({
                "time_pricing": {
                    "timezone": "Asia/Shanghai",
                    "windows": [
                        {"id": "a", "weekdays": ["monday"], "start": "09:00", "end": "12:00", "price_multiplier": 2},
                        {"id": "b", "weekdays": ["monday"], "start": "12:00", "end": "14:00", "price_multiplier": 2}
                    ]
                }
            })),
        )
        .expect("adjacent windows are valid");
    }
}
