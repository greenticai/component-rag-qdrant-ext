//! Configuration: what this extension needs to reach Qdrant and an embeddings
//! API, and how it is assembled for each call. Pure.
//!
//! # Where configuration comes from
//!
//! Two channels, in this order of precedence:
//!
//! 1. **`_tenant_overlay`** — the reserved argument key both hosts stamp onto
//!    every tool call. It carries this extension's *effective* configuration
//!    for the calling tenant: the operator baseline deep-merged with that
//!    tenant's override, resolved from the admin's `extension_config` tables.
//!    It is per-call, per-tenant, and current.
//! 2. **`lifecycle::init`** — a process-wide baseline parsed once and kept in
//!    a `OnceLock`. Optional, and in practice never supplied: the host runtime
//!    exposes `invoke-tool`, `evaluate-guardrail` and `validate-content`, and
//!    no init/configure entry point at all. It stays supported because a host
//!    that grows one would be handing us a legitimate baseline.
//!
//! The overlay wins because it is the more specific and more recent statement
//! of the same thing. `init` runs once when the component is loaded and knows
//! nothing about who is calling; the overlay is resolved by the host for this
//! tenant, on this call, and already *contains* the operator baseline that an
//! `init` would have carried. Letting a load-time value outrank it would mean
//! an operator's change in the admin console silently failed to take effect
//! until the component was reloaded — the failure mode that is hardest to
//! diagnose from a browser.
//!
//! The one exception is `qdrant_url`; see [`resolve`].
//!
//! Neither channel is required to be complete. What each call actually needs
//! is a `qdrant_url` and a collection; everything else has a working default.

use std::sync::OnceLock;

use serde::Deserialize;

use crate::error::RagError;

const DEFAULT_MAX_CHARS: usize = 1200;
const DEFAULT_OVERLAP_CHARS: usize = 150;

/// The OpenAI defaults. `describe.json` already allow-lists
/// `https://api.openai.com/*`, and this is the shape every other field in the
/// embeddings client assumes, so an operator who configures only `qdrant_url`
/// and `collection` gets a working extension instead of a second round of
/// "and now fill in these three too".
const DEFAULT_EMBEDDING_BASE_URL: &str = "https://api.openai.com/v1";
const DEFAULT_EMBEDDING_MODEL: &str = "text-embedding-3-small";
const DEFAULT_EMBEDDING_DIMENSIONS: u32 = 1536;

// ===== resolved configuration =====
//
// These are the *outcome* of resolution: every field present, every field
// validated. They are deliberately not `Deserialize` — nothing arrives in this
// shape, it is only ever built by `resolve`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    /// Base URL of an OpenAI-shaped embeddings API. `/embeddings` is appended.
    pub base_url: String,
    pub model: String,
    pub dimensions: u32,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_EMBEDDING_BASE_URL.to_string(),
            model: DEFAULT_EMBEDDING_MODEL.to_string(),
            dimensions: DEFAULT_EMBEDDING_DIMENSIONS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkConfig {
    pub max_chars: usize,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub qdrant_url: String,
    /// May be empty: neither channel is obliged to name a collection here, and
    /// a tenant-pinned one arrives separately. `ops::collection_of` is the
    /// single authority on which collection a call actually touches, and it
    /// refuses an empty outcome rather than addressing `/collections//...`.
    pub collection: String,
    pub embedding: EmbeddingConfig,
    pub chunk: ChunkConfig,
    /// Refuse any call the host did not stamp a tenant collection onto,
    /// instead of falling back to `collection` above.
    ///
    /// Off by default, because single-tenant installs and local development
    /// have no overlay and must keep working. Multi-tenant operators should
    /// turn it on: without it the guest cannot tell "no overlay because this
    /// install is single-tenant" from "no overlay because the host is too old
    /// to stamp one", and those two fail in opposite directions — the first
    /// harmlessly, the second by serving every tenant the same collection.
    ///
    /// Now that the overlay is also the configuration channel, an unstamped
    /// call usually has no `qdrant_url` either and is refused by [`resolve`]
    /// before this flag is ever consulted. The flag still bites in the two
    /// cases where a baseline *does* exist: an install that also ran
    /// `lifecycle::init`, and an overlay that arrived carrying the operator
    /// baseline but no collection for this tenant.
    pub require_tenant_overlay: bool,
}

// ===== the wire shape =====
//
// Every field optional, every unknown key ignored. Both channels parse into
// this: `lifecycle::init`'s JSON body and the host's `_tenant_overlay` are the
// same document, differing only in scope, so they get the same type and the
// same merge.

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct EmbeddingOverlay {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub dimensions: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ChunkOverlay {
    #[serde(default)]
    pub max_chars: Option<usize>,
    #[serde(default)]
    pub overlap_chars: Option<usize>,
}

/// This extension's configuration as it arrives over either channel.
///
/// Delivered per call under the reserved args key `_tenant_overlay`, and
/// parsed from the JSON body of `lifecycle::init` on the rare host that calls
/// it.
///
/// The per-call copy is emphatically **not** caller input: both hosts strip
/// `_tenant_overlay` from the caller's arguments unconditionally and re-insert
/// their own, including when a tenant has no override configured. That
/// unconditional strip is what makes it trustworthy where the plain
/// `collection` argument never can be — without it, a caller could smuggle an
/// overlay naming another tenant's collection during the window when no
/// override happened to be set.
///
/// Every field is optional and unknown keys are ignored rather than rejected:
/// a host that learns to send more of the blob must not break a guest that has
/// not learned to read it yet, and a tenant override legitimately sets only
/// the one or two fields it changes.
///
/// Deliberately **not** cached in a `static`. The `lifecycle::init` baseline is
/// per-instance; this is per-call, and one instance serves many tenants.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ConfigOverlay {
    #[serde(default)]
    pub qdrant_url: Option<String>,
    /// The collection this tenant's data lives in. When present it is
    /// authoritative — see `ops::collection_of`.
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub embedding: Option<EmbeddingOverlay>,
    #[serde(default)]
    pub chunk: Option<ChunkOverlay>,
    #[serde(default)]
    pub require_tenant_overlay: Option<bool>,
}

/// The error an operator sees when a call arrives with nothing configured.
///
/// This is the message that reaches a browser, so it names the console, the
/// screen and the two fields that actually have to be filled in. The old
/// wording — "lifecycle::init has not run" — described a guest-internal
/// mechanism the reader has no way to act on, and pointed at an entry point no
/// host calls.
pub(crate) fn not_configured(field: &str) -> RagError {
    RagError::InvalidInput(format!(
        "config: no `{field}` is configured for this tenant. Open the admin console, \
         go to Extensions → RAG (Qdrant) → Configuration, and set `qdrant_url` and \
         `collection` — either as the operator baseline or as this tenant's override — \
         then retry."
    ))
}

/// Every URL here is joined with a leading-slash path, so a trailing slash
/// would produce `//collections/...` — accepted by some proxies, 404 by others.
/// Surrounding whitespace goes too, so a field an operator left as spaces
/// normalises to empty and is caught by the emptiness checks rather than being
/// sent as a hostname.
fn trim_slashes(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Merge the `lifecycle::init` baseline (if any) with this call's tenant
/// overlay (if any), then validate the result.
///
/// Precedence is overlay-first for every field, with one exception:
///
/// **`qdrant_url` may not be changed by an overlay once `lifecycle::init` has
/// pinned one.** This is the old cross-cluster refusal, re-derived. It used to
/// be justified by an implementation detail — the request builders read
/// `cfg.qdrant_url` directly, so an overlay URL would have been silently
/// ignored — and that detail is gone: the URL below *is* the one every builder
/// uses. The refusal survives because the real reason was never the builders.
/// One instance holds one Qdrant credential (`secret://rag-qdrant/qdrant_api_key`)
/// under one network allow-list, both instance-wide. Redirecting a tenant to a
/// different cluster would send that instance's credential to a host the
/// operator never named it for. So when an operator has explicitly pinned the
/// cluster for the whole process, a per-tenant contradiction is refused rather
/// than honoured.
///
/// When no `init` ran — which is every real deployment today — there is no
/// contradiction to refuse: the overlay is the *only* statement of where the
/// cluster is, and honouring it is the entire point. An overlay that merely
/// repeats the baseline URL is accepted, since a fully-merged overlay always
/// carries it.
///
/// `collection` is resolved here only far enough to carry a default forward.
/// Which collection a call touches — and whether the caller may name one — is
/// `ops::collection_of`'s decision, because that also depends on the arguments.
///
/// # Errors
/// [`RagError::InvalidInput`] when no `qdrant_url` is configured on either
/// channel, when the overlay's `qdrant_url` is blank or contradicts an
/// `init`-pinned one, or when a merged value is out of range (zero
/// dimensions, zero chunk window, overlap at or above the window).
pub fn resolve(base: Option<&Config>, overlay: Option<&ConfigOverlay>) -> Result<Config, RagError> {
    let embedding_overlay = overlay.and_then(|o| o.embedding.as_ref());
    let chunk_overlay = overlay.and_then(|o| o.chunk.as_ref());

    let qdrant_url = resolve_qdrant_url(base, overlay.and_then(|o| o.qdrant_url.as_deref()))?;

    let collection = overlay
        .and_then(|o| o.collection.clone())
        .or_else(|| base.map(|b| b.collection.clone()))
        .unwrap_or_default();

    let base_embedding = base.map_or_else(EmbeddingConfig::default, |b| b.embedding.clone());
    let embedding = EmbeddingConfig {
        base_url: embedding_overlay
            .and_then(|e| e.base_url.as_deref())
            .map_or(base_embedding.base_url, trim_slashes),
        model: embedding_overlay
            .and_then(|e| e.model.clone())
            .unwrap_or(base_embedding.model),
        dimensions: embedding_overlay
            .and_then(|e| e.dimensions)
            .unwrap_or(base_embedding.dimensions),
    };

    let base_chunk = base.map_or_else(ChunkConfig::default, |b| b.chunk.clone());
    let chunk = ChunkConfig {
        max_chars: chunk_overlay
            .and_then(|c| c.max_chars)
            .unwrap_or(base_chunk.max_chars),
        overlap_chars: chunk_overlay
            .and_then(|c| c.overlap_chars)
            .unwrap_or(base_chunk.overlap_chars),
    };

    let require_tenant_overlay = overlay
        .and_then(|o| o.require_tenant_overlay)
        .or_else(|| base.map(|b| b.require_tenant_overlay))
        .unwrap_or(false);

    if embedding.base_url.is_empty() {
        return Err(RagError::InvalidInput(
            "config: embedding.base_url is empty".into(),
        ));
    }
    if embedding.dimensions == 0 {
        return Err(RagError::InvalidInput(
            "config: embedding.dimensions must be greater than zero".into(),
        ));
    }
    if chunk.max_chars == 0 {
        return Err(RagError::InvalidInput(
            "config: chunk.max_chars must be greater than zero".into(),
        ));
    }
    if chunk.overlap_chars >= chunk.max_chars {
        return Err(RagError::InvalidInput(format!(
            "config: chunk.overlap_chars ({}) must be less than chunk.max_chars ({})",
            chunk.overlap_chars, chunk.max_chars
        )));
    }

    Ok(Config {
        qdrant_url,
        collection,
        embedding,
        chunk,
        require_tenant_overlay,
    })
}

/// See [`resolve`] for why an overlay may supply this URL but may not change
/// one an `init` already pinned.
fn resolve_qdrant_url(
    base: Option<&Config>,
    from_overlay: Option<&str>,
) -> Result<String, RagError> {
    let from_overlay = from_overlay.map(trim_slashes);
    let from_init = base.map(|b| b.qdrant_url.as_str());

    match (from_overlay, from_init) {
        (Some(overlay_url), _) if overlay_url.is_empty() => Err(RagError::InvalidInput(
            "tenant overlay sets an empty qdrant_url; fix this tenant's extension \
             configuration"
                .into(),
        )),
        (Some(overlay_url), Some(init_url)) if overlay_url != init_url => {
            Err(RagError::InvalidInput(format!(
                "tenant overlay sets qdrant_url {overlay_url:?}, which differs from the \
                 {init_url:?} this instance was initialised with. One instance serves one \
                 cluster: its Qdrant credential and network allow-list are instance-wide, so \
                 pointing a tenant at another cluster would send this instance's credential \
                 somewhere the operator never named. Isolate tenants by collection, or run \
                 one instance per cluster."
            )))
        }
        (Some(overlay_url), _) => Ok(overlay_url),
        (None, Some(init_url)) => Ok(init_url.to_string()),
        (None, None) => Err(not_configured("qdrant_url")),
    }
}

/// A `OnceLock<Config>` plus the re-init policy around it, factored out of
/// the free `store`/`installed` functions so the policy — identical reload
/// succeeds, differing reload fails — is testable against a fresh,
/// independent instance instead of the process-wide static below. That
/// keeps the `store()` round-trip tests (finding 4) from racing the
/// process-wide `CONFIG`, which `no_baseline_is_installed_until_init_runs`
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

    fn installed(&self) -> Option<&Config> {
        self.0.get()
    }
}

static CONFIG: ConfigStore = ConfigStore::new();

/// Parse and validate a complete configuration document — the body of
/// `lifecycle::init`.
///
/// Resolving it against no overlay is what validates it, so an `init` that
/// would be unusable on its own still fails at `init` rather than on the first
/// tool call. A baseline that omits `collection` is allowed: the overlay may
/// supply it, and `ops::collection_of` refuses at call time if nothing does.
///
/// # Errors
/// [`RagError::InvalidInput`] naming the offending field. Never panics — a bad
/// config must surface as a failed call, not a trapped component.
pub fn parse_config(json: &str) -> Result<Config, RagError> {
    let overlay: ConfigOverlay =
        serde_json::from_str(json).map_err(|e| RagError::InvalidInput(format!("config: {e}")))?;
    resolve(None, Some(&overlay))
}

/// Store the parsed baseline. Called from `lifecycle::init`. See
/// [`ConfigStore::store`] for the re-init policy.
///
/// # Errors
/// [`RagError::InvalidInput`] if `init` was already called with a config that
/// differs from `cfg`.
pub fn store(cfg: Config) -> Result<(), RagError> {
    CONFIG.store(cfg)
}

/// The `lifecycle::init` baseline, if a host ever supplied one.
///
/// `None` is the normal case, not an error: the host runtime has no
/// init/configure entry point, so configuration arrives per call in the tenant
/// overlay instead. Callers pass this straight to [`resolve`].
#[must_use]
pub fn installed() -> Option<&'static Config> {
    CONFIG.installed()
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

    fn overlay(json: &str) -> ConfigOverlay {
        serde_json::from_str(json).expect("overlay fixture must parse")
    }

    /// Default off: a single-tenant install and local development have no
    /// overlay and must keep working without touching their config.
    #[test]
    fn require_tenant_overlay_defaults_to_off() {
        let cfg = parse_config(FULL).expect("FULL must parse");
        assert!(!cfg.require_tenant_overlay);
    }

    #[test]
    fn require_tenant_overlay_can_be_switched_on() {
        let json = FULL.replace(
            r#""collection": "kb""#,
            r#""collection": "kb", "require_tenant_overlay": true"#,
        );
        let cfg = parse_config(&json).expect("config with the flag must parse");
        assert!(cfg.require_tenant_overlay);
    }

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
    fn no_baseline_is_installed_until_init_runs() {
        // `installed()` on an un-stored OnceLock must return None, never
        // unwrap or fabricate a config. This is a real assertion, not
        // `if let None = ...`, which would pass either way.
        //
        // This depends on nothing in this crate's test binary ever calling
        // the module-level `store()` function, since it writes to the
        // process-wide `CONFIG` static and `cargo test` runs a crate's unit
        // tests in one process (in parallel threads, but sharing that one
        // static). No test does: the `store()` re-init policy (finding 4,
        // below) is tested against fresh, independent `ConfigStore`
        // instances, and the resolution tests pass an explicit
        // `Option<&Config>` baseline, specifically so neither has to touch —
        // or can ever poison — this assertion.
        assert!(installed().is_none());
    }

    // ---- resolution: overlay over baseline ------------------------------

    /// The whole point of this change. No host calls `lifecycle::init`, so
    /// this is the only path a real deployment takes.
    #[test]
    fn an_overlay_alone_configures_the_extension_with_no_init_at_all() {
        let cfg = resolve(
            None,
            Some(&overlay(
                r#"{"qdrant_url":"https://t.qdrant.io:6333","collection":"tenant-a"}"#,
            )),
        )
        .expect("an overlay carrying a url and a collection is a complete configuration");
        assert_eq!(cfg.qdrant_url, "https://t.qdrant.io:6333");
        assert_eq!(cfg.collection, "tenant-a");
        // Everything the overlay left out falls back to a working default.
        assert_eq!(cfg.embedding.base_url, DEFAULT_EMBEDDING_BASE_URL);
        assert_eq!(cfg.embedding.model, DEFAULT_EMBEDDING_MODEL);
        assert_eq!(cfg.embedding.dimensions, DEFAULT_EMBEDDING_DIMENSIONS);
        assert_eq!(cfg.chunk, ChunkConfig::default());
    }

    /// Every field, not just `collection` — an overlay is a whole config.
    #[test]
    fn an_overlay_can_supply_the_embedding_and_chunk_settings_too() {
        let cfg = resolve(
            None,
            Some(&overlay(
                r#"{
                  "qdrant_url":"https://t.qdrant.io:6333",
                  "collection":"tenant-a",
                  "embedding":{"base_url":"https://llm.internal/v1/","model":"e5","dimensions":768},
                  "chunk":{"max_chars":400,"overlap_chars":40}
                }"#,
            )),
        )
        .unwrap();
        assert_eq!(cfg.embedding.base_url, "https://llm.internal/v1");
        assert_eq!(cfg.embedding.model, "e5");
        assert_eq!(cfg.embedding.dimensions, 768);
        assert_eq!(cfg.chunk.max_chars, 400);
        assert_eq!(cfg.chunk.overlap_chars, 40);
    }

    /// `lifecycle::init` stays supported: a baseline with no overlay resolves
    /// to itself, unchanged.
    #[test]
    fn an_init_baseline_alone_still_configures_the_extension() {
        let base = parse_config(FULL).unwrap();
        let resolved = resolve(Some(&base), None).expect("a baseline alone must resolve");
        assert_eq!(resolved, base);
    }

    /// Deep merge, not replace: a tenant that overrides one field keeps the
    /// baseline for the rest.
    #[test]
    fn an_overlay_field_wins_over_the_baseline_and_leaves_its_siblings_alone() {
        let base = parse_config(FULL).unwrap();
        let cfg = resolve(
            Some(&base),
            Some(&overlay(
                r#"{"collection":"tenant-a","embedding":{"dimensions":768}}"#,
            )),
        )
        .unwrap();
        assert_eq!(cfg.collection, "tenant-a");
        assert_eq!(cfg.embedding.dimensions, 768);
        // Untouched by the overlay, so still the baseline's.
        assert_eq!(cfg.embedding.model, "text-embedding-3-small");
        assert_eq!(cfg.qdrant_url, base.qdrant_url);
        assert_eq!(cfg.chunk, base.chunk);
    }

    #[test]
    fn an_overlay_may_switch_require_tenant_overlay_on() {
        let base = parse_config(FULL).unwrap();
        assert!(!base.require_tenant_overlay);
        let cfg = resolve(
            Some(&base),
            Some(&overlay(r#"{"require_tenant_overlay":true}"#)),
        )
        .unwrap();
        assert!(cfg.require_tenant_overlay);
    }

    /// Forward compatibility: a host that learns to send more of the config
    /// must not break a guest that only reads part of it.
    #[test]
    fn unknown_overlay_keys_are_ignored() {
        let cfg = resolve(
            None,
            Some(&overlay(
                r#"{"qdrant_url":"https://t.qdrant.io:6333","collection":"c","future_key":{"a":1}}"#,
            )),
        )
        .unwrap();
        assert_eq!(cfg.collection, "c");
    }

    // ---- resolution: nothing configured ---------------------------------

    #[test]
    fn no_baseline_and_no_overlay_is_an_error_naming_what_to_do_about_it() {
        let err = resolve(None, None).unwrap_err();
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput");
        };
        assert!(msg.contains("qdrant_url"), "message was: {msg}");
        assert!(msg.contains("collection"), "message was: {msg}");
        assert!(msg.contains("admin console"), "message was: {msg}");
        // The old message pointed the operator at a guest-internal entry
        // point no host calls. It must not come back.
        assert!(!msg.contains("lifecycle::init"), "message was: {msg}");
    }

    #[test]
    fn an_overlay_with_no_qdrant_url_and_no_baseline_is_the_same_error() {
        let err = resolve(None, Some(&overlay(r#"{"collection":"tenant-a"}"#))).unwrap_err();
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput");
        };
        assert!(msg.contains("admin console"), "message was: {msg}");
    }

    // ---- resolution: the cross-cluster refusal --------------------------

    /// Re-derived, not dropped. With a baseline pinned by `lifecycle::init`,
    /// a contradicting per-tenant cluster is still refused.
    #[test]
    fn an_overlay_qdrant_url_contradicting_the_init_baseline_is_refused() {
        let base = parse_config(FULL).unwrap();
        let err = resolve(
            Some(&base),
            Some(&overlay(
                r#"{"collection":"tenant-a","qdrant_url":"https://elsewhere.qdrant.io:6333"}"#,
            )),
        )
        .unwrap_err();
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput");
        };
        assert!(msg.contains("elsewhere.qdrant.io"), "message was: {msg}");
        assert!(msg.contains("c.qdrant.io"), "message was: {msg}");
    }

    /// A fully-merged overlay always echoes the baseline url; that must not
    /// trip the refusal, trailing slash or not.
    #[test]
    fn an_overlay_repeating_the_baseline_qdrant_url_is_accepted() {
        let base = parse_config(FULL).unwrap();
        let cfg = resolve(
            Some(&base),
            Some(&overlay(
                r#"{"collection":"tenant-a","qdrant_url":"https://c.qdrant.io:6333/"}"#,
            )),
        )
        .expect("an echo of the baseline is not a contradiction");
        assert_eq!(cfg.qdrant_url, base.qdrant_url);
        assert_eq!(cfg.collection, "tenant-a");
    }

    /// With no `init`, there is nothing for the overlay to contradict — it is
    /// the only statement of where the cluster is, and per-tenant clusters
    /// work.
    #[test]
    fn without_an_init_baseline_the_overlay_chooses_the_cluster_freely() {
        let cfg = resolve(
            None,
            Some(&overlay(
                r#"{"collection":"tenant-a","qdrant_url":"https://elsewhere.qdrant.io:6333"}"#,
            )),
        )
        .expect("no baseline means no contradiction");
        assert_eq!(cfg.qdrant_url, "https://elsewhere.qdrant.io:6333");
    }

    #[test]
    fn a_blank_overlay_qdrant_url_is_refused_rather_than_ignored() {
        let err = resolve(None, Some(&overlay(r#"{"qdrant_url":"   "}"#))).unwrap_err();
        // Whitespace is not a URL, and silently falling back would point the
        // tenant at the operator baseline.
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
    }

    #[test]
    fn an_out_of_range_overlay_value_is_rejected_at_resolution() {
        let base = parse_config(FULL).unwrap();
        let err = resolve(
            Some(&base),
            Some(&overlay(r#"{"embedding":{"dimensions":0}}"#)),
        )
        .unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");

        // Cross-field, and only checkable after the merge: the baseline's
        // overlap against the overlay's window.
        let err = resolve(
            Some(&base),
            Some(&overlay(r#"{"chunk":{"max_chars":100}}"#)),
        )
        .unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
    }

    // ---- the init re-configuration policy -------------------------------

    fn full_cfg() -> Config {
        parse_config(FULL).unwrap()
    }

    #[test]
    fn a_config_store_accepts_its_first_config() {
        let store = ConfigStore::new();
        assert!(store.store(full_cfg()).is_ok());
        assert_eq!(store.installed().unwrap().collection, "kb");
    }

    #[test]
    fn a_second_store_of_an_identical_config_is_a_harmless_reload() {
        let store = ConfigStore::new();
        store.store(full_cfg()).unwrap();
        // Same fields, freshly parsed — a distinct `Config` value that
        // compares equal, not the same one reused.
        assert!(store.store(full_cfg()).is_ok());
        assert_eq!(store.installed().unwrap().collection, "kb");
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
        assert_eq!(store.installed().unwrap().collection, "kb");
    }
}
