//! DateTime node - date and time manipulation utilities.

use async_trait::async_trait;
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Timelike, Utc,
};
use serde::Deserialize;
use serde_json::{json, Value};

use super::types::{Node, NodeContext, NodeResult};
use crate::error::{Error, Result};

/// DateTime node for date/time operations.
pub struct DateTimeNode;

impl DateTimeNode {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DateTimeNode {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DateTimeOperation {
    Now,
    Parse,
    Format,
    Add,
    Subtract,
    Diff,
    StartOf,
    EndOf,
    Compare,
    Extract,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TimeUnit {
    Years,
    Months,
    Weeks,
    Days,
    Hours,
    Minutes,
    Seconds,
    Milliseconds,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PeriodBoundary {
    Year,
    Month,
    Week,
    Day,
    Hour,
    Minute,
}

#[derive(Debug, Deserialize)]
struct DateTimeConfig {
    /// The operation to perform
    operation: DateTimeOperation,

    /// Input date/time string (ISO 8601 format)
    #[serde(default)]
    date: Option<String>,

    /// Field path in input to get date from
    #[serde(default)]
    date_field: Option<String>,

    /// Second date for diff/compare operations
    #[serde(default)]
    date2: Option<String>,

    /// Second date field path
    #[serde(default)]
    date2_field: Option<String>,

    /// Amount for add/subtract operations
    #[serde(default)]
    amount: Option<i64>,

    /// Unit for add/subtract/diff operations
    #[serde(default)]
    unit: Option<TimeUnit>,

    /// Period boundary for start_of/end_of
    #[serde(default)]
    period: Option<PeriodBoundary>,

    /// Output format string (strftime-style)
    #[serde(default)]
    format: Option<String>,

    /// Input format for parsing non-ISO dates
    #[serde(default)]
    input_format: Option<String>,

    /// Timezone (e.g., "UTC", "+05:30")
    #[serde(default)]
    timezone: Option<String>,

    /// What to extract (year, month, day, hour, minute, second, weekday, etc.)
    #[serde(default)]
    extract: Option<String>,
}

#[async_trait]
impl Node for DateTimeNode {
    fn node_type(&self) -> &str {
        "datetime"
    }

    fn description(&self) -> &str {
        "Date and time manipulation: parse, format, add, subtract, compare"
    }

    async fn execute(&self, config: &Value, ctx: &NodeContext) -> Result<NodeResult> {
        let config: DateTimeConfig = serde_json::from_value(config.clone())
            .map_err(|e| Error::Node(format!("Invalid datetime config: {}", e)))?;

        let result = match config.operation {
            DateTimeOperation::Now => {
                let now = Utc::now();
                format_datetime_with_timezone(
                    &now,
                    config.format.as_deref(),
                    config.timezone.as_deref(),
                )?
            }
            DateTimeOperation::Parse => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                format_datetime_with_timezone(
                    &dt,
                    config.format.as_deref(),
                    config.timezone.as_deref(),
                )?
            }
            DateTimeOperation::Format => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let format = config.format.ok_or_else(|| {
                    Error::Node("Format operation requires 'format' field".to_string())
                })?;
                let dt = apply_timezone(&dt, config.timezone.as_deref())?;
                dt.format(&format).to_string()
            }
            DateTimeOperation::Add => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let amount = config.amount.ok_or_else(|| {
                    Error::Node("Add operation requires 'amount' field".to_string())
                })?;
                let unit = config.unit.ok_or_else(|| {
                    Error::Node("Add operation requires 'unit' field".to_string())
                })?;
                let result_dt = add_duration(&dt, amount, &unit)?;
                format_datetime_with_timezone(
                    &result_dt,
                    config.format.as_deref(),
                    config.timezone.as_deref(),
                )?
            }
            DateTimeOperation::Subtract => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let amount = config.amount.ok_or_else(|| {
                    Error::Node("Subtract operation requires 'amount' field".to_string())
                })?;
                let unit = config.unit.ok_or_else(|| {
                    Error::Node("Subtract operation requires 'unit' field".to_string())
                })?;
                let result_dt = add_duration(&dt, -amount, &unit)?;
                format_datetime_with_timezone(
                    &result_dt,
                    config.format.as_deref(),
                    config.timezone.as_deref(),
                )?
            }
            DateTimeOperation::Diff => {
                let date1_str = get_date_string(&config, &ctx.input, false)?;
                let date2_str = get_date_string(&config, &ctx.input, true)?;
                let dt1 = parse_datetime(
                    &date1_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let dt2 = parse_datetime(
                    &date2_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let unit = config.unit.unwrap_or(TimeUnit::Seconds);
                let diff = calculate_diff(&dt1, &dt2, &unit);
                return Ok(NodeResult::with_metadata(
                    json!({ "result": diff }),
                    json!({ "operation": "diff", "unit": format!("{:?}", unit) }),
                ));
            }
            DateTimeOperation::StartOf => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let period = config.period.ok_or_else(|| {
                    Error::Node("StartOf operation requires 'period' field".to_string())
                })?;
                let result_dt = start_of(&dt, &period);
                format_datetime_with_timezone(
                    &result_dt,
                    config.format.as_deref(),
                    config.timezone.as_deref(),
                )?
            }
            DateTimeOperation::EndOf => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let period = config.period.ok_or_else(|| {
                    Error::Node("EndOf operation requires 'period' field".to_string())
                })?;
                let result_dt = end_of(&dt, &period);
                format_datetime_with_timezone(
                    &result_dt,
                    config.format.as_deref(),
                    config.timezone.as_deref(),
                )?
            }
            DateTimeOperation::Compare => {
                let date1_str = get_date_string(&config, &ctx.input, false)?;
                let date2_str = get_date_string(&config, &ctx.input, true)?;
                let dt1 = parse_datetime(
                    &date1_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let dt2 = parse_datetime(
                    &date2_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;

                let comparison = if dt1 < dt2 {
                    "before"
                } else if dt1 > dt2 {
                    "after"
                } else {
                    "equal"
                };

                return Ok(NodeResult::with_metadata(
                    json!({
                        "result": comparison,
                        "is_before": dt1 < dt2,
                        "is_after": dt1 > dt2,
                        "is_equal": dt1 == dt2,
                    }),
                    json!({ "operation": "compare" }),
                ));
            }
            DateTimeOperation::Extract => {
                let date_str = get_date_string(&config, &ctx.input, false)?;
                let dt = parse_datetime(
                    &date_str,
                    config.input_format.as_deref(),
                    config.timezone.as_deref(),
                )?;
                let extract = config.extract.ok_or_else(|| {
                    Error::Node("Extract operation requires 'extract' field".to_string())
                })?;
                let dt = apply_timezone(&dt, config.timezone.as_deref())?;
                let value = extract_component(&dt, &extract)?;
                return Ok(NodeResult::with_metadata(
                    json!({ "result": value }),
                    json!({ "operation": "extract", "component": extract }),
                ));
            }
        };

        Ok(NodeResult::with_metadata(
            json!({ "result": result }),
            json!({ "operation": format!("{:?}", config.operation) }),
        ))
    }
}

/// Get date string from config or input.
fn get_date_string(config: &DateTimeConfig, input: &Value, use_date2: bool) -> Result<String> {
    let (date_opt, field_opt) = if use_date2 {
        (&config.date2, &config.date2_field)
    } else {
        (&config.date, &config.date_field)
    };

    if let Some(date) = date_opt {
        return Ok(date.clone());
    }

    if let Some(field) = field_opt {
        return get_field_value_as_string(input, field);
    }

    // Try to get from input directly
    match input {
        Value::String(s) => Ok(s.clone()),
        Value::Object(obj) => {
            let field_name = if use_date2 { "date2" } else { "date" };
            if let Some(Value::String(s)) = obj.get(field_name) {
                return Ok(s.clone());
            }
            Err(Error::Node(format!(
                "No {} found in input. Specify '{}' or '{}_field' in config",
                field_name, field_name, field_name
            )))
        }
        _ => Err(Error::Node("Cannot extract date from input".to_string())),
    }
}

/// Get nested field value as string.
fn get_field_value_as_string(value: &Value, field_path: &str) -> Result<String> {
    let mut current = value.clone();
    for part in field_path.split('.') {
        current = match current {
            Value::Object(obj) => obj
                .get(part)
                .cloned()
                .ok_or_else(|| Error::Node(format!("Field '{}' not found", field_path)))?,
            _ => return Err(Error::Node(format!("Cannot index into {:?}", current))),
        };
    }

    match current {
        Value::String(s) => Ok(s),
        other => Ok(other.to_string()),
    }
}

/// Parse datetime from string.
fn parse_datetime(s: &str, format: Option<&str>, timezone: Option<&str>) -> Result<DateTime<Utc>> {
    if let Some(fmt) = format {
        // Try parsing with custom format
        let naive = NaiveDateTime::parse_from_str(s, fmt)
            .or_else(|_| {
                // Try parsing as date only
                NaiveDate::parse_from_str(s, fmt).map(|d| d.and_hms_opt(0, 0, 0).unwrap())
            })
            .map_err(|e| {
                Error::Node(format!(
                    "Failed to parse date '{}' with format '{}': {}",
                    s, fmt, e
                ))
            })?;
        let tz = parse_timezone_offset(timezone)?;
        let local = tz.from_local_datetime(&naive).single().ok_or_else(|| {
            Error::Node(format!("Ambiguous local datetime for timezone '{}'", tz))
        })?;
        return Ok(local.with_timezone(&Utc));
    }

    // Try ISO 8601 formats
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try ISO 8601 without timezone
    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        let tz = parse_timezone_offset(timezone)?;
        let local = tz.from_local_datetime(&naive).single().ok_or_else(|| {
            Error::Node(format!("Ambiguous local datetime for timezone '{}'", tz))
        })?;
        return Ok(local.with_timezone(&Utc));
    }

    // Try date only
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let tz = parse_timezone_offset(timezone)?;
        let naive = date.and_hms_opt(0, 0, 0).unwrap();
        let local = tz.from_local_datetime(&naive).single().ok_or_else(|| {
            Error::Node(format!("Ambiguous local datetime for timezone '{}'", tz))
        })?;
        return Ok(local.with_timezone(&Utc));
    }

    Err(Error::Node(format!(
        "Failed to parse date '{}'. Use ISO 8601 format or specify 'input_format'",
        s
    )))
}

/// Format datetime to string.
fn format_datetime(dt: &DateTime<Utc>, format: Option<&str>) -> String {
    match format {
        Some(fmt) => dt.format(fmt).to_string(),
        None => dt.to_rfc3339(),
    }
}

fn parse_timezone_offset(timezone: Option<&str>) -> Result<FixedOffset> {
    let tz = timezone.unwrap_or("UTC").trim();
    if tz.eq_ignore_ascii_case("utc") || tz == "Z" {
        return FixedOffset::east_opt(0)
            .ok_or_else(|| Error::Node("Invalid UTC offset".to_string()));
    }

    let sign = if tz.starts_with('+') {
        1
    } else if tz.starts_with('-') {
        -1
    } else {
        return Err(Error::Node(format!(
            "Invalid timezone '{}'. Use UTC or offset like +08:00",
            tz
        )));
    };

    let tz = &tz[1..];
    let (hours, minutes) = if let Some((h, m)) = tz.split_once(':') {
        (h.parse::<i32>(), m.parse::<i32>())
    } else if tz.len() == 4 {
        (tz[0..2].parse::<i32>(), tz[2..4].parse::<i32>())
    } else {
        return Err(Error::Node(format!(
            "Invalid timezone '{}'. Use +HH:MM or -HHMM format",
            timezone.unwrap_or("")
        )));
    };

    let hours = hours.map_err(|_| {
        Error::Node(format!(
            "Invalid timezone hour in '{}'",
            timezone.unwrap_or("")
        ))
    })?;
    let minutes = minutes.map_err(|_| {
        Error::Node(format!(
            "Invalid timezone minute in '{}'",
            timezone.unwrap_or("")
        ))
    })?;

    if hours > 23 || minutes > 59 {
        return Err(Error::Node(format!(
            "Invalid timezone '{}'. Hour must be <= 23 and minute <= 59",
            timezone.unwrap_or("")
        )));
    }

    let total_seconds = sign * (hours * 3600 + minutes * 60);
    FixedOffset::east_opt(total_seconds)
        .ok_or_else(|| Error::Node("Invalid timezone offset".to_string()))
}

fn apply_timezone(dt: &DateTime<Utc>, timezone: Option<&str>) -> Result<DateTime<FixedOffset>> {
    let tz = parse_timezone_offset(timezone)?;
    Ok(dt.with_timezone(&tz))
}

fn format_datetime_with_timezone(
    dt: &DateTime<Utc>,
    format: Option<&str>,
    timezone: Option<&str>,
) -> Result<String> {
    if timezone.is_none()
        || timezone
            .map(|tz| {
                let trimmed = tz.trim();
                trimmed.eq_ignore_ascii_case("utc") || trimmed == "Z"
            })
            .unwrap_or(false)
    {
        return Ok(format_datetime(dt, format));
    }

    let dt = apply_timezone(dt, timezone)?;
    match format {
        Some(fmt) => Ok(dt.format(fmt).to_string()),
        None => Ok(dt.to_rfc3339()),
    }
}

/// Add duration to datetime.
fn add_duration(dt: &DateTime<Utc>, amount: i64, unit: &TimeUnit) -> Result<DateTime<Utc>> {
    let duration = match unit {
        TimeUnit::Years => {
            // Approximate: 365 days per year
            Duration::days(amount * 365)
        }
        TimeUnit::Months => {
            // Approximate: 30 days per month
            Duration::days(amount * 30)
        }
        TimeUnit::Weeks => Duration::weeks(amount),
        TimeUnit::Days => Duration::days(amount),
        TimeUnit::Hours => Duration::hours(amount),
        TimeUnit::Minutes => Duration::minutes(amount),
        TimeUnit::Seconds => Duration::seconds(amount),
        TimeUnit::Milliseconds => Duration::milliseconds(amount),
    };

    dt.checked_add_signed(duration)
        .ok_or_else(|| Error::Node("Date overflow".to_string()))
}

/// Calculate difference between two datetimes.
fn calculate_diff(dt1: &DateTime<Utc>, dt2: &DateTime<Utc>, unit: &TimeUnit) -> i64 {
    let duration = *dt2 - *dt1;

    match unit {
        TimeUnit::Years => duration.num_days() / 365,
        TimeUnit::Months => duration.num_days() / 30,
        TimeUnit::Weeks => duration.num_weeks(),
        TimeUnit::Days => duration.num_days(),
        TimeUnit::Hours => duration.num_hours(),
        TimeUnit::Minutes => duration.num_minutes(),
        TimeUnit::Seconds => duration.num_seconds(),
        TimeUnit::Milliseconds => duration.num_milliseconds(),
    }
}

/// Get start of period.
fn start_of(dt: &DateTime<Utc>, period: &PeriodBoundary) -> DateTime<Utc> {
    match period {
        PeriodBoundary::Year => Utc
            .with_ymd_and_hms(dt.year(), 1, 1, 0, 0, 0)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Month => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), 1, 0, 0, 0)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Week => {
            let days_from_monday = dt.weekday().num_days_from_monday();
            let start = *dt - Duration::days(days_from_monday as i64);
            Utc.with_ymd_and_hms(start.year(), start.month(), start.day(), 0, 0, 0)
                .single()
                .unwrap_or(*dt)
        }
        PeriodBoundary::Day => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 0, 0, 0)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Hour => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 0, 0)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Minute => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 0)
            .single()
            .unwrap_or(*dt),
    }
}

/// Get end of period.
fn end_of(dt: &DateTime<Utc>, period: &PeriodBoundary) -> DateTime<Utc> {
    match period {
        PeriodBoundary::Year => Utc
            .with_ymd_and_hms(dt.year(), 12, 31, 23, 59, 59)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Month => {
            let next_month = if dt.month() == 12 {
                Utc.with_ymd_and_hms(dt.year() + 1, 1, 1, 0, 0, 0)
            } else {
                Utc.with_ymd_and_hms(dt.year(), dt.month() + 1, 1, 0, 0, 0)
            };
            next_month.single().unwrap_or(*dt) - Duration::seconds(1)
        }
        PeriodBoundary::Week => {
            let days_to_sunday = 6 - dt.weekday().num_days_from_monday();
            let end = *dt + Duration::days(days_to_sunday as i64);
            Utc.with_ymd_and_hms(end.year(), end.month(), end.day(), 23, 59, 59)
                .single()
                .unwrap_or(*dt)
        }
        PeriodBoundary::Day => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), 23, 59, 59)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Hour => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), 59, 59)
            .single()
            .unwrap_or(*dt),
        PeriodBoundary::Minute => Utc
            .with_ymd_and_hms(dt.year(), dt.month(), dt.day(), dt.hour(), dt.minute(), 59)
            .single()
            .unwrap_or(*dt),
    }
}

/// Extract component from datetime.
fn extract_component<Tz>(dt: &DateTime<Tz>, component: &str) -> Result<Value>
where
    Tz: TimeZone,
    Tz::Offset: std::fmt::Display,
{
    match component.to_lowercase().as_str() {
        "year" => Ok(json!(dt.year())),
        "month" => Ok(json!(dt.month())),
        "day" => Ok(json!(dt.day())),
        "hour" => Ok(json!(dt.hour())),
        "minute" => Ok(json!(dt.minute())),
        "second" => Ok(json!(dt.second())),
        "millisecond" => Ok(json!(dt.timestamp_subsec_millis())),
        "weekday" | "day_of_week" => Ok(json!(dt.weekday().num_days_from_monday())),
        "weekday_name" => Ok(json!(dt.weekday().to_string())),
        "day_of_year" => Ok(json!(dt.ordinal())),
        "week_of_year" | "iso_week" => Ok(json!(dt.iso_week().week())),
        "quarter" => Ok(json!((dt.month() - 1) / 3 + 1)),
        "timestamp" | "unix" => Ok(json!(dt.timestamp())),
        "timestamp_millis" => Ok(json!(dt.timestamp_millis())),
        "is_leap_year" => Ok(json!(
            dt.year() % 4 == 0 && (dt.year() % 100 != 0 || dt.year() % 400 == 0)
        )),
        _ => Err(Error::Node(format!("Unknown component: {}", component))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_iso8601() {
        let dt = parse_datetime("2024-01-15T10:30:00Z", None, None).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 10);
        assert_eq!(dt.minute(), 30);
    }

    #[test]
    fn test_parse_date_only() {
        let dt = parse_datetime("2024-01-15", None, None).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
        assert_eq!(dt.hour(), 0);
    }

    #[test]
    fn test_parse_custom_format() {
        let dt = parse_datetime("15/01/2024", Some("%d/%m/%Y"), None).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 15);
    }

    #[test]
    fn test_parse_datetime_with_timezone_offset() {
        let dt = parse_datetime("2024-01-15T00:00:00", None, Some("+08:00")).unwrap();
        assert_eq!(dt.year(), 2024);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 14);
        assert_eq!(dt.hour(), 16);
    }

    #[test]
    fn test_format_datetime_with_timezone_offset() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let formatted = format_datetime_with_timezone(&dt, Some("%H:%M"), Some("+08:00")).unwrap();
        assert_eq!(formatted, "08:00");
    }

    #[test]
    fn test_parse_timezone_offset_invalid() {
        let err = parse_timezone_offset(Some("GMT+8")).unwrap_err();
        assert!(err.to_string().contains("Invalid timezone"));
    }

    #[test]
    fn test_format_datetime() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let formatted = format_datetime(&dt, Some("%Y-%m-%d"));
        assert_eq!(formatted, "2024-01-15");
    }

    #[test]
    fn test_add_days() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let result = add_duration(&dt, 5, &TimeUnit::Days).unwrap();
        assert_eq!(result.day(), 20);
    }

    #[test]
    fn test_add_hours() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let result = add_duration(&dt, 3, &TimeUnit::Hours).unwrap();
        assert_eq!(result.hour(), 13);
    }

    #[test]
    fn test_subtract_days() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let result = add_duration(&dt, -5, &TimeUnit::Days).unwrap();
        assert_eq!(result.day(), 10);
    }

    #[test]
    fn test_diff_days() {
        let dt1 = Utc.with_ymd_and_hms(2024, 1, 10, 0, 0, 0).unwrap();
        let dt2 = Utc.with_ymd_and_hms(2024, 1, 15, 0, 0, 0).unwrap();
        let diff = calculate_diff(&dt1, &dt2, &TimeUnit::Days);
        assert_eq!(diff, 5);
    }

    #[test]
    fn test_diff_hours() {
        let dt1 = Utc.with_ymd_and_hms(2024, 1, 15, 10, 0, 0).unwrap();
        let dt2 = Utc.with_ymd_and_hms(2024, 1, 15, 15, 0, 0).unwrap();
        let diff = calculate_diff(&dt1, &dt2, &TimeUnit::Hours);
        assert_eq!(diff, 5);
    }

    #[test]
    fn test_start_of_day() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 45).unwrap();
        let start = start_of(&dt, &PeriodBoundary::Day);
        assert_eq!(start.hour(), 0);
        assert_eq!(start.minute(), 0);
        assert_eq!(start.second(), 0);
    }

    #[test]
    fn test_start_of_month() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let start = start_of(&dt, &PeriodBoundary::Month);
        assert_eq!(start.day(), 1);
        assert_eq!(start.hour(), 0);
    }

    #[test]
    fn test_end_of_day() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let end = end_of(&dt, &PeriodBoundary::Day);
        assert_eq!(end.hour(), 23);
        assert_eq!(end.minute(), 59);
        assert_eq!(end.second(), 59);
    }

    #[test]
    fn test_extract_year() {
        let dt = Utc.with_ymd_and_hms(2024, 1, 15, 10, 30, 0).unwrap();
        let value = extract_component(&dt, "year").unwrap();
        assert_eq!(value, json!(2024));
    }

    #[test]
    fn test_extract_quarter() {
        let dt = Utc.with_ymd_and_hms(2024, 7, 15, 10, 30, 0).unwrap();
        let value = extract_component(&dt, "quarter").unwrap();
        assert_eq!(value, json!(3));
    }

    #[tokio::test]
    async fn test_datetime_node_now() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "now"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert!(result.data["result"].is_string());
    }

    #[tokio::test]
    async fn test_datetime_node_parse() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "parse",
            "date": "2024-01-15T10:30:00Z"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        let parsed = result.data["result"].as_str().unwrap();
        assert!(parsed.contains("2024-01-15"));
    }

    #[tokio::test]
    async fn test_datetime_node_format() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "format",
            "date": "2024-01-15T10:30:00Z",
            "format": "%Y-%m-%d"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["result"], "2024-01-15");
    }

    #[tokio::test]
    async fn test_datetime_node_add() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "add",
            "date": "2024-01-15T10:30:00Z",
            "amount": 5,
            "unit": "days",
            "format": "%Y-%m-%d"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["result"], "2024-01-20");
    }

    #[tokio::test]
    async fn test_datetime_node_diff() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "diff",
            "date": "2024-01-10T00:00:00Z",
            "date2": "2024-01-15T00:00:00Z",
            "unit": "days"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["result"], 5);
    }

    #[tokio::test]
    async fn test_datetime_node_compare() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "compare",
            "date": "2024-01-10T00:00:00Z",
            "date2": "2024-01-15T00:00:00Z"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["result"], "before");
        assert_eq!(result.data["is_before"], true);
        assert_eq!(result.data["is_after"], false);
    }

    #[tokio::test]
    async fn test_datetime_node_extract() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "extract",
            "date": "2024-07-15T10:30:00Z",
            "extract": "quarter"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["result"], 3);
    }

    #[tokio::test]
    async fn test_datetime_node_start_of() {
        let node = DateTimeNode::new();
        let config = json!({
            "operation": "start_of",
            "date": "2024-01-15T10:30:45Z",
            "period": "day",
            "format": "%Y-%m-%dT%H:%M:%S"
        });
        let ctx = NodeContext::new("exec-1", "test");

        let result = node.execute(&config, &ctx).await.unwrap();
        assert_eq!(result.data["result"], "2024-01-15T00:00:00");
    }
}
