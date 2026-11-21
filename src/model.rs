use serde::Deserialize;
use serde_json::Value;

/// Root object produced by `opencode session export <id>`.
#[derive(Debug, Deserialize)]
pub struct Export {
    pub info: Info,
    #[serde(default)]
    pub messages: Vec<Message>,
}

#[derive(Debug, Deserialize)]
pub struct Info {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "projectID", default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<Model>,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub tokens: Option<Tokens>,
    #[serde(default)]
    pub time: Option<InfoTime>,
    #[serde(default)]
    pub location: Option<Location>,
}

#[derive(Debug, Deserialize)]
pub struct Model {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "providerID", default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub variant: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Tokens {
    #[serde(default)]
    pub input: Option<u64>,
    #[serde(default)]
    pub output: Option<u64>,
    #[serde(default)]
    pub reasoning: Option<u64>,
    #[serde(default)]
    pub cache: Option<Cache>,
}

#[derive(Debug, Default, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub read: Option<u64>,
    #[serde(default)]
    pub write: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InfoTime {
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub updated: Option<i64>,
    #[serde(default)]
    pub busy: Option<i64>,
}

impl InfoTime {
    /// Total wall-clock duration of the session in milliseconds.
    pub fn wall(&self) -> Option<i64> {
        let start = self.created?;
        let end = self.updated.or(self.busy)?;
        Some(end.saturating_sub(start))
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct Location {
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub time: Option<MessageTime>,
    /// User prompt text.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub files: Vec<Value>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub model: Option<Model>,
    /// Assistant content parts (text, reasoning, tool, file, ...).
    #[serde(default)]
    pub content: Vec<Value>,
    #[serde(default)]
    pub cost: Option<f64>,
    #[serde(default)]
    pub tokens: Option<Tokens>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MessageTime {
    #[serde(default)]
    pub created: Option<i64>,
    #[serde(default)]
    pub completed: Option<i64>,
}
