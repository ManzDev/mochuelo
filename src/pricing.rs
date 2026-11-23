use crate::model::{Export, Message, Model};
use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

/// USD prices per million tokens.
#[derive(Debug, Clone, Copy)]
struct Rates {
    input: f64,
    cache_read: f64,
    cache_write: f64,
    output: f64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Costs {
    pub input_cached: f64,
    pub input_uncached: f64,
    pub output_uncached: f64,
    pub input_cached_tokens: u64,
    pub input_uncached_tokens: u64,
    pub output_uncached_tokens: u64,
}

impl Costs {
    pub fn total(&self) -> f64 {
        self.input_cached + self.input_uncached + self.output_uncached
    }
}

pub struct Pricing {
    rates: HashMap<(String, String), Rates>,
}

impl Pricing {
    /// Load pricing for the models used by this export from OpenCode's cached
    /// models.dev catalog. No network access.
    pub fn load(export: &Export) -> Self {
        let needed = used_models(export);
        if needed.is_empty() {
            return Self {
                rates: HashMap::new(),
            };
        }

        let rates = load_rates(&needed).unwrap_or_default();
        Self { rates }
    }

    /// Load every rate in the cached models.dev catalog. Used by the global
    /// statistics view, where sessions can use many different models.
    pub fn load_all() -> Self {
        Self {
            rates: load_all_rates().unwrap_or_default(),
        }
    }

    /// Estimate cost for arbitrary token counts (USD per million token rates).
    pub fn estimate(
        &self,
        provider: &str,
        model: &str,
        input: u64,
        cache_read: u64,
        cache_write: u64,
        output: u64,
    ) -> Option<f64> {
        let rate = self.rate(provider, model)?;
        Some(
            (input as f64 * rate.input
                + cache_read as f64 * rate.cache_read
                + cache_write as f64 * rate.cache_write
                + output as f64 * rate.output)
                / 1_000_000.0,
        )
    }

    fn rate(&self, provider: &str, model: &str) -> Option<Rates> {
        self.rates
            .get(&(provider.to_string(), model.to_string()))
            .copied()
    }
}

/// Compute the cost split for the session. The bool is false when some model
/// pricing could not be resolved (the numbers are then partial).
pub fn breakdown(export: &Export, pricing: &Pricing) -> (Costs, bool) {
    let mut costs = Costs::default();
    let mut complete = true;

    for message in &export.messages {
        if message.kind != "assistant" {
            continue;
        }
        let Some(tokens) = &message.tokens else {
            continue;
        };
        let model = message.model.as_ref().or(export.info.model.as_ref());
        let Some(model) = model else {
            complete = false;
            continue;
        };
        let (Some(provider), Some(id)) = (model.provider_id.as_deref(), model.id.as_deref())
        else {
            complete = false;
            continue;
        };
        let Some(rate) = pricing.rate(provider, id) else {
            complete = false;
            continue;
        };

        let input = tokens.input.unwrap_or(0) as f64;
        let output = (tokens.output.unwrap_or(0) + tokens.reasoning.unwrap_or(0)) as f64;
        let cache = tokens.cache.as_ref();
        let cache_read = cache.and_then(|c| c.read).unwrap_or(0) as f64;
        let cache_write = cache.and_then(|c| c.write).unwrap_or(0) as f64;

        costs.input_uncached += input * rate.input / 1_000_000.0;
        costs.input_cached +=
            (cache_read * rate.cache_read + cache_write * rate.cache_write) / 1_000_000.0;
        costs.output_uncached += output * rate.output / 1_000_000.0;

        costs.input_uncached_tokens += tokens.input.unwrap_or(0);
        costs.input_cached_tokens += tokens
            .cache
            .as_ref()
            .map(|c| c.read.unwrap_or(0) + c.write.unwrap_or(0))
            .unwrap_or(0);
        costs.output_uncached_tokens +=
            tokens.output.unwrap_or(0) + tokens.reasoning.unwrap_or(0);
    }

    (costs, complete)
}

/// Cost of one assistant message using the catalog rates. Used when OpenCode
/// did not record a per-message cost.
pub fn message_cost(
    message: &Message,
    fallback_model: Option<&Model>,
    pricing: &Pricing,
) -> Option<f64> {
    let tokens = message.tokens.as_ref()?;
    let model = message.model.as_ref().or(fallback_model)?;
    let provider = model.provider_id.as_deref()?;
    let id = model.id.as_deref()?;
    let rate = pricing.rate(provider, id)?;

    let input = tokens.input.unwrap_or(0) as f64;
    let output = (tokens.output.unwrap_or(0) + tokens.reasoning.unwrap_or(0)) as f64;
    let cache = tokens.cache.as_ref();
    let cache_read = cache.and_then(|c| c.read).unwrap_or(0) as f64;
    let cache_write = cache.and_then(|c| c.write).unwrap_or(0) as f64;

    Some(
        (input * rate.input
            + cache_read * rate.cache_read
            + cache_write * rate.cache_write
            + output * rate.output)
            / 1_000_000.0,
    )
}

fn used_models(export: &Export) -> HashSet<(String, String)> {
    let mut models = HashSet::new();

    let mut push = |model: &Option<crate::model::Model>| {
        if let Some(model) = model {
            if let (Some(provider), Some(id)) =
                (model.provider_id.as_deref(), model.id.as_deref())
            {
                models.insert((provider.to_string(), id.to_string()));
            }
        }
    };

    for message in &export.messages {
        if message.kind == "assistant" {
            push(&message.model);
        }
    }
    push(&export.info.model);

    models
}

fn load_rates(
    needed: &HashSet<(String, String)>,
) -> Result<HashMap<(String, String), Rates>> {
    let body = catalog_body()?;

    let mut out = HashMap::new();
    for (provider_id, model_id) in needed {
        let Some(provider) = body.get(provider_id) else {
            continue;
        };
        let Some(model) = provider.models.get(model_id) else {
            continue;
        };
        let Some(cost) = model.cost.as_ref().and_then(|cost| cost.text.as_ref()) else {
            continue;
        };
        if let Some(rates) = Rates::from_cost(cost) {
            out.insert((provider_id.clone(), model_id.clone()), rates);
        }
    }

    Ok(out)
}

fn load_all_rates() -> Result<HashMap<(String, String), Rates>> {
    let body = catalog_body()?;
    let mut out = HashMap::new();

    for (provider_id, provider) in &body {
        for (model_id, model) in &provider.models {
            let Some(cost) = model.cost.as_ref().and_then(|cost| cost.text.as_ref()) else {
                continue;
            };
            if let Some(rates) = Rates::from_cost(cost) {
                out.insert((provider_id.clone(), model_id.clone()), rates);
            }
        }
    }

    Ok(out)
}

fn catalog_body() -> Result<HashMap<String, Provider>> {
    let path = crate::sessions::database_path()
        .context("could not locate the OpenCode database for model pricing")?;

    let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open {} read-only", path.display()))?;
    conn.busy_timeout(Duration::from_millis(800))?;

    let raw: String = conn
        .query_row(
            "SELECT value FROM kv WHERE key = 'models-dev:v2:catalog'",
            [],
            |row| row.get(0),
        )
        .context("models.dev catalog not cached by OpenCode")?;
    let catalog: Catalog = serde_json::from_str(&raw)
        .context("failed to parse the cached models.dev catalog")?;

    serde_json::from_str(&catalog.body).context("failed to parse the models.dev catalog body")
}

#[derive(Deserialize)]
struct Catalog {
    body: String,
}

#[derive(Deserialize)]
struct Provider {
    #[serde(default)]
    models: HashMap<String, CatalogModel>,
}

#[derive(Deserialize)]
struct CatalogModel {
    #[serde(default)]
    cost: Option<ModelCost>,
}

#[derive(Deserialize)]
struct ModelCost {
    #[serde(default)]
    text: Option<TextCost>,
}

#[derive(Deserialize)]
struct TextCost {
    #[serde(default)]
    input: Option<InputCost>,
    #[serde(default)]
    output: Option<OutputCost>,
}

#[derive(Deserialize)]
struct InputCost {
    #[serde(default)]
    price: Option<f64>,
    #[serde(default)]
    cache: Option<CacheCost>,
}

#[derive(Deserialize)]
struct OutputCost {
    #[serde(default)]
    price: Option<f64>,
}

#[derive(Deserialize)]
struct CacheCost {
    #[serde(default)]
    read: Option<Rate>,
    #[serde(default)]
    write: Option<Rate>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Rate {
    Number(f64),
    Map(HashMap<String, f64>),
}

impl Rate {
    fn value(&self) -> f64 {
        match self {
            Rate::Number(value) => *value,
            Rate::Map(values) => values
                .values()
                .copied()
                .reduce(f64::min)
                .unwrap_or(0.0),
        }
    }
}

impl Rates {
    fn from_cost(cost: &TextCost) -> Option<Self> {
        let input = cost.input.as_ref()?;
        let output = cost.output.as_ref()?.price?;
        let input_price = input.price?;

        let (cache_read, cache_write) = match &input.cache {
            Some(cache) => (
                cache.read.as_ref().map(Rate::value).unwrap_or(input_price),
                cache.write.as_ref().map(Rate::value).unwrap_or(input_price),
            ),
            None => (input_price, input_price),
        };

        Some(Self {
            input: input_price,
            cache_read,
            cache_write,
            output,
        })
    }
}
