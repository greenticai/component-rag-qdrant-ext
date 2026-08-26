//! Operator configuration, parsed once in `lifecycle::init`. Pure.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::error::RagError;

const DEFAULT_MAX_CHARS: usize = 1200;
const DEFAULT_OVERLAP_CHARS: usize = 150;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct EmbeddingConfig {
    /// Base URL of an OpenAI-shaped embeddings API. `/embeddings` is appended.
    pub base_url: String,
    pub model: String,
    pub dimensions: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Config {
    pub qdrant_url: String,
    pub collection: String,
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub chunk: ChunkConfig,
}

/// A `OnceLock<Config>` plus the re-init policy around it, factored out of
/// the free `store`/`current` functions so the policy — identical reload
/// succeeds, differing reload fails — is testable against a fresh,
/// independent instance instead of the process-wide static below. That
/// keeps the `store()` round-trip tests (finding 4) from racing the
/// process-wide `CONFIG`, which `reading_config_before_init_is_an_error_not_a_panic`
/// depends on staying unset for the life of the test binary.
struct ConfigStore(OnceLock<Config>);

impl ConfigStore {
    const fn new() -> Self {
        Self(OnceLock::new())
    }

    /// A host may call `init` more than once — on a reload, a re-enable, or
    /// an operator config change. A `OnceLock` cannot be replaced, so a
    /// second call with the *same* config is treated as a harmless reload
    /// and succeeds; a second call with a *different* config cannot be
    /// honoured (the running instance would still use the old settings) and
    /// must not be silently swallowed, so it fails loudly instead.
    fn store(&self, cfg: Config) -> Result<(), RagError> {
        if let Some(existing) = self.0.get() {
            return if *existing == cfg {
                Ok(())
            } else {
                Err(RagError::InvalidInput(
                    "config: extension is already configured with different settings — \
                     restart the extension to change them"
                        .into(),
                ))
            };
        }
        self.0
            .set(cfg)
            .map_err(|_| RagError::InvalidInput("config: init called more than once".into()))
    }

    fn current(&self) -> Result<&Config, RagError> {
        self.0.get().ok_or_else(|| {
            RagError::InvalidInput(
                "config: extension is not configured — lifecycle::init has not run".into(),
            )
        })
    }
}

static CONFIG: ConfigStore = ConfigStore::new();

/// Parse and validate operator configuration.
///
/// # Errors
/// [`RagError::InvalidInput`] naming the offending field. Never panics — a bad
/// config must surface as a failed call, not a trapped component.
pub fn parse_config(json: &str) -> Result<Config, RagError> {
    let mut cfg: Config =
        serde_json::from_str(json).map_err(|e| RagError::InvalidInput(format!("config: {e}")))?;

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

/// Store the parsed config. Called from `lifecycle::init`. See
/// [`ConfigStore::store`] for the re-init policy.
///
/// # Errors
/// [`RagError::InvalidInput`] if `init` was already called with a config that
/// differs from `cfg`.
pub fn store(cfg: Config) -> Result<(), RagError> {
    CONFIG.store(cfg)
}

/// Borrow the stored config.
///
/// # Errors
/// [`RagError::InvalidInput`] when `lifecycle::init` has not run. Hosts are not
/// required to call `init` before `invoke_tool`, so this is a real path.
pub fn current() -> Result<&'static Config, RagError> {
    CONFIG.current()
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
        // This is a real assertion, not `if let Err(e) = ...`, which would
        // pass even if `current()` wrongly returned a config.
        //
        // This depends on nothing in this crate's test binary ever calling
        // the module-level `store()` function, since it writes to the
        // process-wide `CONFIG` static and `cargo test` runs a crate's unit
        // tests in one process (in parallel threads, but sharing that one
        // static). No test does: the `store()` re-init policy (finding 4,
        // below) is tested against fresh, independent `ConfigStore`
        // instances instead of the process-wide one, specifically so it
        // never has to touch — and can never poison — this assertion.
        assert!(matches!(current(), Err(RagError::InvalidInput(_))));
    }

    fn full_cfg() -> Config {
        parse_config(FULL).unwrap()
    }

    #[test]
    fn a_config_store_accepts_its_first_config() {
        let store = ConfigStore::new();
        assert!(store.store(full_cfg()).is_ok());
        assert_eq!(store.current().unwrap().collection, "kb");
    }

    #[test]
    fn a_second_store_of_an_identical_config_is_a_harmless_reload() {
        let store = ConfigStore::new();
        store.store(full_cfg()).unwrap();
        // Same fields, freshly parsed — a distinct `Config` value that
        // compares equal, not the same one reused.
        assert!(store.store(full_cfg()).is_ok());
        assert_eq!(store.current().unwrap().collection, "kb");
    }

    #[test]
    fn a_second_store_of_a_differing_config_is_rejected_and_the_original_survives() {
        let store = ConfigStore::new();
        store.store(full_cfg()).unwrap();

        let mut changed = full_cfg();
        changed.collection = "other".to_string();
        let err = store
            .store(changed)
            .expect_err("a changed config must be rejected");
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput");
        };
        assert!(msg.contains("different"), "message was: {msg}");
        assert!(msg.contains("restart"), "message was: {msg}");

        // The OnceLock can't be replaced, so the original config must still
        // be the one in effect — the rejected call must not have discarded
        // it silently.
        assert_eq!(store.current().unwrap().collection, "kb");
    }
}
