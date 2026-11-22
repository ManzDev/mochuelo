use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
    #[serde(default)]
    pub directory: Option<String>,
}

impl Session {
    pub fn title(&self) -> &str {
        let title = self.title.trim();
        if title.is_empty() {
            "(untitled)"
        } else {
            title
        }
    }

    /// Local creation date, or last update, formatted for the picker.
    pub fn when(&self) -> String {
        let stamp = self.created.or(self.updated);
        match stamp.and_then(chrono::DateTime::from_timestamp_millis) {
            Some(time) => time
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            None => "—".to_string(),
        }
    }
}

/// List top-level sessions for the current project, newest first.
pub fn list() -> Result<Vec<Session>> {
    let output = Command::new("opencode")
        .args(["session", "list", "--format", "json"])
        .output()
        .context("failed to run `opencode` — is the OpenCode CLI in your PATH?")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if stderr.is_empty() {
            anyhow::bail!("`opencode session list` failed ({})", output.status);
        }
        anyhow::bail!(
            "`opencode session list` failed ({}):\n{stderr}",
            output.status
        );
    }

    let raw = String::from_utf8_lossy(&output.stdout);
    if raw.trim().is_empty() {
        return Ok(Vec::new());
    }

    serde_json::from_str(raw.trim()).context("failed to parse `opencode session list` JSON")
}

/// List top-level sessions across every project known to OpenCode.
pub fn list_all() -> Result<Vec<Session>> {
    let path = database_path()
        .context("could not locate the OpenCode database (is the `opencode` CLI available?)")?;

    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {} read-only", path.display()))?;
    conn.busy_timeout(Duration::from_millis(800))
        .context("failed to configure the OpenCode database connection")?;

    let mut stmt = conn
        .prepare(
            "SELECT id, COALESCE(title, ''), directory, time_created, time_updated
             FROM session_v2
             WHERE parent_id IS NULL AND time_archived IS NULL
             ORDER BY time_created DESC
             LIMIT 500",
        )
        .context("failed to query sessions (unsupported OpenCode database schema?)")?;

    let rows = stmt
        .query_map([], |row| {
            Ok(Session {
                id: row.get(0)?,
                title: row.get(1)?,
                directory: Some(row.get(2)?),
                created: Some(row.get(3)?),
                updated: Some(row.get(4)?),
            })
        })
        .context("failed to read sessions from the OpenCode database")?;

    let mut sessions = Vec::new();
    for row in rows {
        sessions.push(row.context("failed to read a session row")?);
    }
    Ok(sessions)
}

fn database_path() -> Option<PathBuf> {
    if let Some(path) = debug_db_path() {
        if path.is_file() {
            return Some(path);
        }
    }

    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        })?;

    let candidate = data_home.join("opencode/opencode.db");
    candidate.is_file().then_some(candidate)
}

fn debug_db_path() -> Option<PathBuf> {
    let output = Command::new("opencode")
        .args(["debug", "paths"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("db") {
            let rest = rest.trim();
            if !rest.is_empty() {
                return Some(PathBuf::from(rest));
            }
        }
    }
    None
}
