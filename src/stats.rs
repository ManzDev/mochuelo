use crate::pricing::Pricing;
use anyhow::{Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use rusqlite::{params, Connection, OpenFlags};
use serde::Deserialize;
use std::time::Duration as StdDuration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Global,
    Monthly,
    Weekly,
    Daily,
}

impl Period {
    pub const ALL: [Period; 4] = [
        Period::Global,
        Period::Monthly,
        Period::Weekly,
        Period::Daily,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Period::Global => "Global",
            Period::Monthly => "Monthly",
            Period::Weekly => "Weekly",
            Period::Daily => "Daily",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Period::Global => Period::Monthly,
            Period::Monthly => Period::Weekly,
            Period::Weekly => Period::Daily,
            Period::Daily => Period::Global,
        }
    }

    pub fn previous(self) -> Self {
        match self {
            Period::Global => Period::Daily,
            Period::Monthly => Period::Global,
            Period::Weekly => Period::Monthly,
            Period::Daily => Period::Weekly,
        }
    }

    /// Inclusive start / exclusive end, plus a human-readable label. `offset`
    /// walks backwards from the current month/week/day (0 = current period).
    pub fn range(self, offset: i64) -> Option<PeriodRange> {
        let today = Local::now().date_naive();

        match self {
            Period::Global => Some(PeriodRange {
                label: "All available history".to_string(),
                short_label: "All available history".to_string(),
                start: None,
                end: None,
            }),
            Period::Monthly => {
                let base = NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?;
                let start_date = shift_months(base, -i32::try_from(offset).ok()?)?;
                let end_date = shift_months(start_date, 1)?;
                let label = start_date.format("%B %Y").to_string();
                Some(PeriodRange {
                    label: label.clone(),
                    short_label: label,
                    start: Some(midnight_ms(start_date)),
                    end: Some(midnight_ms(end_date)),
                })
            }
            Period::Weekly => {
                let monday =
                    today - Duration::days(today.weekday().num_days_from_monday() as i64);
                let start_date = monday - Duration::weeks(offset);
                let end_date = start_date + Duration::weeks(1);
                let sunday = end_date - Duration::days(1);
                let short_label = format!(
                    "{}-{}",
                    compact_day(start_date),
                    compact_day(sunday)
                );
                Some(PeriodRange {
                    label: format!(
                        "Week {}-{} {}",
                        start_date.format("%d/%b"),
                        sunday.format("%d/%b"),
                        start_date.format("%Y")
                    ),
                    short_label,
                    start: Some(midnight_ms(start_date)),
                    end: Some(midnight_ms(end_date)),
                })
            }
            Period::Daily => {
                let start_date = today - Duration::days(offset);
                let end_date = start_date + Duration::days(1);
                Some(PeriodRange {
                    label: start_date.format("%A, %d %b %Y").to_string(),
                    short_label: start_date.format("%d/%b").to_string(),
                    start: Some(midnight_ms(start_date)),
                    end: Some(midnight_ms(end_date)),
                })
            }
        }
    }
}

pub struct PeriodRange {
    pub label: String,
    pub short_label: String,
    pub start: Option<i64>,
    pub end: Option<i64>,
}

#[derive(Clone, Copy, Default)]
pub struct GlobalStats {
    pub sessions: u64,
    pub messages: u64,
    pub input_total: u64,
    pub cached_input: u64,
    pub output_total: u64,
    pub cost: f64,
    pub duration_ms: i64,
}

impl GlobalStats {
    pub fn total_tokens(&self) -> u64 {
        self.input_total + self.output_total
    }

    pub fn uncached_input(&self) -> u64 {
        self.input_total.saturating_sub(self.cached_input)
    }

    /// How many input tokens were processed per uncached input token.
    pub fn cache_multiplier(&self) -> Option<f64> {
        let uncached = self.uncached_input();
        if uncached == 0 {
            None
        } else {
            Some(self.input_total as f64 / uncached as f64)
        }
    }

    pub fn cost_per_million(&self) -> f64 {
        let total = self.total_tokens();
        if total == 0 {
            0.0
        } else {
            self.cost / total as f64 * 1_000_000.0
        }
    }
}

/// Aggregate usage for all sessions in the selected period.
pub fn load(
    period: Period,
    offset: i64,
    pricing: &Pricing,
) -> Result<(String, GlobalStats)> {
    let range = period.range(offset).context("invalid period")?;
    let path = crate::sessions::database_path()
        .context("could not locate the OpenCode database")?;

    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {} read-only", path.display()))?;
    conn.busy_timeout(StdDuration::from_millis(800))?;

    let mut stats = GlobalStats::default();
    let mut stmt = conn.prepare(
        "SELECT model, cost, tokens_input, tokens_output, tokens_reasoning,
                tokens_cache_read, tokens_cache_write, time_updated - time_created
         FROM session_v2
         WHERE time_archived IS NULL
           AND (?1 IS NULL OR (time_created >= ?1 AND time_created < ?2))",
    )?;

    let rows = stmt.query_map(params![range.start, range.end], |row| {
        Ok(SessionRow {
            model: row.get(0)?,
            cost: row.get(1)?,
            input: row.get(2)?,
            output: row.get(3)?,
            reasoning: row.get(4)?,
            cache_read: row.get(5)?,
            cache_write: row.get(6)?,
            duration: row.get(7)?,
        })
    })?;

    for row in rows {
        let row = row?;
        let input = row.input.unwrap_or(0).max(0) as u64;
        let output = row.output.unwrap_or(0).max(0) as u64;
        let reasoning = row.reasoning.unwrap_or(0).max(0) as u64;
        let cache_read = row.cache_read.unwrap_or(0).max(0) as u64;
        let cache_write = row.cache_write.unwrap_or(0).max(0) as u64;

        stats.sessions += 1;
        stats.input_total += input + cache_read + cache_write;
        stats.cached_input += cache_read + cache_write;
        stats.output_total += output + reasoning;
        stats.duration_ms += row.duration.unwrap_or(0).max(0);

        let recorded = row.cost.unwrap_or(0.0);
        stats.cost += if recorded > 0.0 {
            recorded
        } else {
            estimate_session_cost(
                row.model.as_deref(),
                input,
                cache_read,
                cache_write,
                output + reasoning,
                pricing,
            )
            .unwrap_or(0.0)
        };
    }

    stats.messages = conn.query_row(
        "SELECT COUNT(*)
         FROM session_message
         WHERE session_id IN (
             SELECT id FROM session_v2
             WHERE time_archived IS NULL
               AND (?1 IS NULL OR (time_created >= ?1 AND time_created < ?2))
         )",
        params![range.start, range.end],
        |row| row.get::<_, i64>(0),
    )? as u64;

    Ok((range.label, stats))
}

struct SessionRow {
    model: Option<String>,
    cost: Option<f64>,
    input: Option<i64>,
    output: Option<i64>,
    reasoning: Option<i64>,
    cache_read: Option<i64>,
    cache_write: Option<i64>,
    duration: Option<i64>,
}

fn estimate_session_cost(
    model: Option<&str>,
    input: u64,
    cache_read: u64,
    cache_write: u64,
    output: u64,
    pricing: &Pricing,
) -> Option<f64> {
    #[derive(Deserialize)]
    struct SessionModel {
        id: Option<String>,
        #[serde(rename = "providerID")]
        provider_id: Option<String>,
    }

    let model: SessionModel = serde_json::from_str(model?).ok()?;
    pricing.estimate(
        model.provider_id.as_deref()?,
        model.id.as_deref()?,
        input,
        cache_read,
        cache_write,
        output,
    )
}

fn compact_day(date: NaiveDate) -> String {
    let month = date.format("%b").to_string();
    let letter = month.chars().next().unwrap_or('?');
    format!("{}{}", date.day(), letter)
}

fn midnight_ms(date: NaiveDate) -> i64 {
    let naive = date.and_hms_opt(0, 0, 0).expect("valid midnight");
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|time| time.timestamp_millis())
        .unwrap_or_else(|| naive.and_utc().timestamp_millis())
}

fn shift_months(date: NaiveDate, delta: i32) -> Option<NaiveDate> {
    let month_index = date.year() * 12 + date.month0() as i32 + delta;
    let year = month_index.div_euclid(12);
    let month = month_index.rem_euclid(12) as u32 + 1;
    NaiveDate::from_ymd_opt(year, month, 1)
}
