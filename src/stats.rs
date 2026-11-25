use crate::pricing::Pricing;
use anyhow::{Context, Result};
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::HashSet;
use std::time::Duration as StdDuration;

/// Gaps longer than this between messages are treated as inactivity and capped.
const IDLE_GAP_CAP_MS: i64 = 5 * 60 * 1000;

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

pub struct ActivityBucket {
    pub offset: i64,
    pub tokens: u64,
}

impl Period {
    fn activity_cap(self) -> i64 {
        match self {
            Period::Global => 0,
            Period::Monthly => 24,
            Period::Weekly => 52,
            Period::Daily => 90,
        }
    }

    fn bucket_count(self, oldest: NaiveDate, today: NaiveDate) -> i64 {
        match self {
            Period::Global => 0,
            Period::Monthly => {
                let oldest_index = oldest.year() * 12 + oldest.month0() as i32;
                let today_index = today.year() * 12 + today.month0() as i32;
                (today_index - oldest_index + 1) as i64
            }
            Period::Weekly => {
                let oldest_monday =
                    oldest - Duration::days(oldest.weekday().num_days_from_monday() as i64);
                let today_monday =
                    today - Duration::days(today.weekday().num_days_from_monday() as i64);
                ((today_monday - oldest_monday).num_days() / 7 + 1) as i64
            }
            Period::Daily => (today - oldest).num_days() + 1,
        }
    }
}

fn open_database() -> Result<Connection> {
    let path = crate::sessions::database_path()
        .context("could not locate the OpenCode database")?;
    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {} read-only", path.display()))?;
    conn.busy_timeout(StdDuration::from_millis(800))?;
    Ok(conn)
}

struct MessageRow {
    session_id: String,
    time: i64,
    previous: Option<i64>,
    /// User and assistant rows delimit real activity; idle/system events do not.
    timed: bool,
    tokens: Option<MessageTokens>,
    cost: Option<f64>,
    model: Option<MessageModel>,
}

#[derive(Deserialize)]
struct MessageModel {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "providerID")]
    provider_id: Option<String>,
}

#[derive(Default)]
struct MessageTokens {
    input: u64,
    output: u64,
    reasoning: u64,
    cache_read: u64,
    cache_write: u64,
}

impl MessageTokens {
    fn total(&self) -> u64 {
        self.input + self.cache_read + self.cache_write + self.output + self.reasoning
    }
}

/// Loads every message once, extracting only the fields used by the stats view.
/// Parsing the full JSON payload would be slow: message content can be huge.
fn load_message_rows(conn: &Connection) -> Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(
        "SELECT sm.session_id, sm.type, sm.time_created,
                LAG(sm.time_created) OVER (PARTITION BY sm.session_id ORDER BY sm.seq),
                json_extract(sm.data, '$.tokens.input'),
                json_extract(sm.data, '$.tokens.output'),
                json_extract(sm.data, '$.tokens.reasoning'),
                json_extract(sm.data, '$.tokens.cache.read'),
                json_extract(sm.data, '$.tokens.cache.write'),
                json_extract(sm.data, '$.cost'),
                json_extract(sm.data, '$.model.id'),
                json_extract(sm.data, '$.model.providerID')
         FROM session_message sm
         JOIN session_v2 s ON s.id = sm.session_id
         WHERE s.time_archived IS NULL
         ORDER BY sm.session_id, sm.seq",
    )?;

    let rows = stmt.query_map([], |row| {
        let kind: String = row.get(1)?;
        let tokens = if kind == "assistant" {
            Some(MessageTokens {
                input: row.get::<_, Option<u64>>(4)?.unwrap_or(0),
                output: row.get::<_, Option<u64>>(5)?.unwrap_or(0),
                reasoning: row.get::<_, Option<u64>>(6)?.unwrap_or(0),
                cache_read: row.get::<_, Option<u64>>(7)?.unwrap_or(0),
                cache_write: row.get::<_, Option<u64>>(8)?.unwrap_or(0),
            })
        } else {
            None
        };
        let model = match (row.get::<_, Option<String>>(10)?, row.get::<_, Option<String>>(11)?) {
            (None, None) => None,
            (id, provider_id) => Some(MessageModel { id, provider_id }),
        };

        Ok(MessageRow {
            session_id: row.get(0)?,
            time: row.get(2)?,
            previous: row.get(3)?,
            timed: kind == "user" || kind == "assistant",
            tokens,
            cost: row.get(9)?,
            model,
        })
    })?;

    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    Ok(messages)
}

fn activity_from_rows(period: Period, rows: &[MessageRow]) -> Vec<ActivityBucket> {
    if period == Period::Global || rows.is_empty() {
        return Vec::new();
    }

    let today = Local::now().date_naive();
    let oldest = rows
        .iter()
        .map(|row| row.time)
        .min()
        .unwrap_or_else(|| Local::now().timestamp_millis());
    let oldest_date = Local
        .timestamp_millis_opt(oldest)
        .single()
        .map(|time| time.date_naive())
        .unwrap_or(today);
    let count = period.bucket_count(oldest_date, today);
    if count <= 0 {
        return Vec::new();
    }

    let cap = period.activity_cap();
    let max_offset = (count - 1).min(cap - 1);
    let mut values = Vec::new();

    for offset in (0..=max_offset).rev() {
        let Some(range) = period.range(offset) else {
            continue;
        };
        let (Some(start), Some(end)) = (range.start, range.end) else {
            continue;
        };

        let tokens: u64 = rows
            .iter()
            .filter(|row| row.time >= start && row.time < end)
            .filter_map(|row| row.tokens.as_ref())
            .map(MessageTokens::total)
            .sum();
        values.push(ActivityBucket { offset, tokens });
    }

    values
}

fn in_range(start: Option<i64>, end: Option<i64>, time: i64) -> bool {
    match (start, end) {
        (Some(start), Some(end)) => time >= start && time < end,
        _ => true,
    }
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

/// Aggregate usage for all messages created in the selected period.
pub fn load(
    period: Period,
    offset: i64,
    pricing: &Pricing,
) -> Result<(String, GlobalStats, Vec<ActivityBucket>)> {
    let range = period.range(offset).context("invalid period")?;
    let conn = open_database()?;
    let rows = load_message_rows(&conn)?;

    let mut stats = GlobalStats::default();
    let mut sessions = HashSet::new();

    for row in &rows {
        if !in_range(range.start, range.end, row.time) {
            continue;
        }

        stats.messages += 1;
        sessions.insert(row.session_id.as_str());

        if row.timed {
            if let Some(previous) = row.previous {
                let gap = row.time - previous;
                if gap > 0 {
                    stats.duration_ms += gap.min(IDLE_GAP_CAP_MS);
                }
            }
        }

        let Some(tokens) = &row.tokens else {
            continue;
        };

        stats.input_total += tokens.input + tokens.cache_read + tokens.cache_write;
        stats.cached_input += tokens.cache_read + tokens.cache_write;
        stats.output_total += tokens.output + tokens.reasoning;

        stats.cost += match row.cost {
            Some(recorded) if recorded > 0.0 => recorded,
            _ => {
                estimate_message_cost(row.model.as_ref(), tokens, pricing).unwrap_or(0.0)
            }
        };
    }

    stats.sessions = sessions.len() as u64;
    let activity = activity_from_rows(period, &rows);

    Ok((range.label, stats, activity))
}

fn estimate_message_cost(
    model: Option<&MessageModel>,
    tokens: &MessageTokens,
    pricing: &Pricing,
) -> Option<f64> {
    let model = model?;
    pricing.estimate(
        model.provider_id.as_deref()?,
        model.id.as_deref()?,
        tokens.input,
        tokens.cache_read,
        tokens.cache_write,
        tokens.output + tokens.reasoning,
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
