//! Operator configuration, parsed once in `lifecycle::init`. Pure.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::error::RagError;

const DEFAULT_MAX_CHARS: usize = 1200;
const DEFAULT_OVERLAP_CHARS: usize = 150;

#[derive(Debug, Clone, Deserialize)]
pub struct EmbeddingConfig {
    /// Base URL of an OpenAI-shaped embeddings API. `/embeddings` is appended.
    pub base_url: String,
    pub model: String,
    pub dimensions: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChunkConfig {
    #[serde(default = "default_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_overlap_chars")]
    pub overlap_chars: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chars: DEFAULT_MAX_CHARS,
            overlap_chars: DEFAULT_OVERLAP_CHARS,
        }
    }
}

fn default_max_chars() -> usize {
    DEFAULT_MAX_CHARS
}

fn default_overlap_chars() -> usize {
    DEFAULT_OVERLAP_CHARS
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub qdrant_url: String,
    pub collection: String,
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub chunk: ChunkConfig,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

/// Parse and validate operator configuration.
///
/// # Errors
/// [`RagError::InvalidInput`] naming the offending field. Never panics — a bad
/// config must surface as a failed call, not a trapped component.
pub fn parse_config(json: &str) -> Result<Config, RagError> {
    let mut cfg: Config = serde_json::from_str(json)
        .map_err(|e| RagError::InvalidInput(format!("config: {e}")))?;

    // Every URL is joined with a leading-slash path, so a trailing slash here
    // would produce `//collections/...` — accepted by some proxies, 404 by others.
    while cfg.qdrant_url.ends_with('/') {
        cfg.qdrant_url.pop();
    }
    while cfg.embedding.base_url.ends_with('/') {
        cfg.embedding.base_url.pop();
    }

    if cfg.qdrant_url.is_empty() {
        return Err(RagError::InvalidInput("config: qdrant_url is empty".into()));
    }
    if cfg.collection.is_empty() {
        return Err(RagError::InvalidInput("config: collection is empty".into()));
    }
    if cfg.embedding.dimensions == 0 {
        return Err(RagError::InvalidInput(
            "config: embedding.dimensions must be greater than zero".into(),
        ));
    }
    if cfg.chunk.max_chars == 0 {
        return Err(RagError::InvalidInput(
            "config: chunk.max_chars must be greater than zero".into(),
        ));
    }
    if cfg.chunk.overlap_chars >= cfg.chunk.max_chars {
        return Err(RagError::InvalidInput(format!(
            "config: chunk.overlap_chars ({}) must be less than chunk.max_chars ({})",
            cfg.chunk.overlap_chars, cfg.chunk.max_chars
        )));
    }
    Ok(cfg)
}

/// Store the parsed config. Called once from `lifecycle::init`.
///
/// # Errors
/// [`RagError::InvalidInput`] if `init` was already called.
pub fn store(cfg: Config) -> Result<(), RagError> {
    CONFIG
        .set(cfg)
        .map_err(|_| RagError::InvalidInput("config: init called more than once".into()))
}

/// Borrow the stored config.
///
/// # Errors
/// [`RagError::InvalidInput`] when `lifecycle::init` has not run. Hosts are not
/// required to call `init` before `invoke_tool`, so this is a real path.
pub fn current() -> Result<&'static Config, RagError> {
    CONFIG.get().ok_or_else(|| {
        RagError::InvalidInput(
            "config: extension is not configured — lifecycle::init has not run".into(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"{
      "qdrant_url": "https://c.qdrant.io:6333",
      "collection": "kb",
      "embedding": {
        "base_url": "https://api.openai.com/v1",
        "model": "text-embedding-3-small",
        "dimensions": 1536
      },
      "chunk": { "max_chars": 1200, "overlap_chars": 150 }
    }"#;

    #[test]
    fn a_full_config_parses() {
        let cfg = parse_config(FULL).unwrap();
        assert_eq!(cfg.qdrant_url, "https://c.qdrant.io:6333");
        assert_eq!(cfg.collection, "kb");
        assert_eq!(cfg.embedding.dimensions, 1536);
        assert_eq!(cfg.chunk.max_chars, 1200);
    }

    #[test]
    fn a_trailing_slash_on_the_qdrant_url_is_normalised_away() {
        let cfg = parse_config(&FULL.replace("6333", "6333/")).unwrap();
        assert!(!cfg.qdrant_url.ends_with('/'));
    }

    #[test]
    fn chunk_settings_default_when_omitted() {
        let json = r#"{
          "qdrant_url": "https://c.qdrant.io:6333",
          "collection": "kb",
          "embedding": { "base_url": "https://x/v1", "model": "m", "dimensions": 8 }
        }"#;
        let cfg = parse_config(json).unwrap();
        assert_eq!(cfg.chunk.max_chars, 1200);
        assert_eq!(cfg.chunk.overlap_chars, 150);
    }

    #[test]
    fn a_missing_required_field_names_the_field() {
        let err = parse_config(r#"{"collection":"kb"}"#).unwrap_err();
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput");
        };
        assert!(msg.contains("qdrant_url"), "message was: {msg}");
    }

    #[test]
    fn an_overlap_at_or_above_the_window_is_rejected() {
        let json = FULL.replace(r#""overlap_chars": 150"#, r#""overlap_chars": 1200"#);
        assert!(matches!(
            parse_config(&json),
            Err(RagError::InvalidInput(_))
        ));
    }

    #[test]
    fn zero_dimensions_is_rejected() {
        let json = FULL.replace(r#""dimensions": 1536"#, r#""dimensions": 0"#);
        assert!(matches!(
            parse_config(&json),
            Err(RagError::InvalidInput(_))
        ));
    }

    #[test]
    fn reading_config_before_init_is_an_error_not_a_panic() {
        // `current()` on an un-stored OnceLock must return Err, never unwrap.
        // This test asserts the shape of the error only; ordering against
        // `store()` is not guaranteed across tests in one binary.
        if let Err(e) = current() {
            assert!(matches!(e, RagError::InvalidInput(_)));
        }
    }
}
