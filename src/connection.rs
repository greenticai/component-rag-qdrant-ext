//! Which Qdrant a call talks to, and under which physical collection name.
//!
//! Precedence, decided once per call:
//!
//! 1. the connected `vector-store/qdrant` infrastructure resource
//!    (`secret://infra/vector_store_qdrant`);
//! 2. only when the host says that resource is **not connected**: the legacy
//!    overlay (`qdrant_url` + `secret://rag-qdrant/qdrant_api_key`);
//! 3. otherwise a named error. There is no default host, ever.
//!
//! A resource that is present but unreadable, malformed or of an unsupported
//! version is an error: it never falls back to the legacy source, because a
//! broken credential must not quietly route traffic to a different cluster.
//!
//! No error produced here contains the credential or the raw document.

use crate::config::Config;
use crate::error::RagError;
use crate::host::HostCalls;
use crate::infra::{self, InfraConnection, InfraError};

/// Legacy secret URI (kept for installs that have not connected a resource).
pub const LEGACY_KEY_REF: &str = "secret://rag-qdrant/qdrant_api_key";

const PREFIX_KEY: &str = "collection_prefix";
const ISOLATION_KEY: &str = "isolation";
const DEDICATED: &str = "dedicated";
const MAX_PREFIX_LEN: usize = 64;
const MAX_NAME_LEN: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Infra,
    Legacy,
}

pub struct Connection {
    pub base_url: String,
    /// Only the request builders read this (as the `api-key` header).
    pub api_key: String,
    /// `Some` only for [`Source::Infra`] on a shared (prefixed) instance.
    pub collection_prefix: Option<String>,
    pub source: Source,
}

impl core::fmt::Debug for Connection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Connection")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("collection_prefix", &self.collection_prefix)
            .field("source", &self.source)
            .finish()
    }
}

impl Connection {
    /// The collection name to put on the wire for a logical one. Only the
    /// infra source is namespaced; the legacy source keeps today's names.
    ///
    /// # Errors
    /// [`RagError::InvalidInput`] for a name that could escape the namespace.
    pub fn physical(&self, logical: &str) -> Result<String, RagError> {
        match self.source {
            Source::Infra => physical_collection(self.collection_prefix.as_deref(), logical),
            Source::Legacy => Ok(logical.to_string()),
        }
    }
}

impl Connection {
    /// `host:port` of the Qdrant being called (no scheme, path or query), for
    /// diagnosing a wrong host or port without printing a URL.
    #[must_use]
    pub fn host_port(&self) -> String {
        let rest = self
            .base_url
            .split_once("://")
            .map_or(self.base_url.as_str(), |(_, r)| r);
        let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
        authority.rsplit('@').next().unwrap_or("").to_string()
    }

    /// How an error should name the thing that failed.
    #[must_use]
    pub fn target(&self) -> String {
        match self.source {
            Source::Infra => format!("the {} resource at {}", infra::KIND, self.host_port()),
            Source::Legacy => format!("Qdrant at {} (legacy configuration)", self.host_port()),
        }
    }

    /// Say WHICH credential Qdrant rejected, and scrub the key from the text.
    #[must_use]
    pub fn attribute(&self, err: RagError) -> RagError {
        match err {
            RagError::PermissionDenied(m) if m.starts_with(crate::qdrant::REJECTED_PREFIX) => {
                let detail = m
                    .strip_prefix(crate::qdrant::REJECTED_PREFIX)
                    .unwrap_or_default();
                let who = match self.source {
                    Source::Infra => format!(
                        "the connected resource's credential was rejected by {}",
                        self.target()
                    ),
                    Source::Legacy => format!(
                        "the legacy credential {LEGACY_KEY_REF} was rejected by {}",
                        self.target()
                    ),
                };
                RagError::PermissionDenied(redact(&format!("{who}{detail}"), &self.api_key))
            }
            other => other,
        }
    }
}

fn name_char_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// The single place a prefix is applied. The logical name must be
/// `[A-Za-z0-9._-]{1,128}` without a leading `__`; the prefix follows the same
/// character rule and is at most 64 characters. A logical name that already
/// starts with the prefix is just a name (it is NOT de-duplicated).
///
/// # Errors
/// [`RagError::InvalidInput`].
pub fn physical_collection(prefix: Option<&str>, logical: &str) -> Result<String, RagError> {
    if logical.is_empty()
        || logical.len() > MAX_NAME_LEN
        || logical.starts_with("__")
        || !logical.chars().all(name_char_ok)
        || logical == "."
        || logical == ".."
    {
        return Err(RagError::InvalidInput(
            "collection name must be 1-128 characters of letters, digits, `.`, `_` or `-`, \
             and must not start with `__`"
                .into(),
        ));
    }
    match prefix {
        None => Ok(logical.to_string()),
        Some(p) => {
            validate_prefix(p)?;
            Ok(format!("{p}{logical}"))
        }
    }
}

fn validate_prefix(prefix: &str) -> Result<(), RagError> {
    if prefix.is_empty() || prefix.len() > MAX_PREFIX_LEN || !prefix.chars().all(name_char_ok) {
        return Err(RagError::InvalidInput(format!(
            "the {} resource's `{PREFIX_KEY}` must be 1-{MAX_PREFIX_LEN} characters of letters, \
             digits, `.`, `_` or `-`",
            infra::KIND
        )));
    }
    Ok(())
}

/// Replace every occurrence of `secret` in `msg`. No-op for an empty secret.
#[must_use]
pub fn redact(msg: &str, secret: &str) -> String {
    if secret.is_empty() {
        return msg.to_string();
    }
    msg.replace(secret, "<redacted>")
}

/// Scrub every non-empty secret in `secrets` from `msg`.
#[must_use]
pub fn redact_all(msg: &str, secrets: &[&str]) -> String {
    secrets.iter().fold(msg.to_string(), |m, s| redact(&m, s))
}

fn infra_error(err: InfraError) -> RagError {
    match err {
        InfraError::Unreadable => RagError::Internal(err.to_string()),
        other => RagError::InvalidInput(other.to_string()),
    }
}

/// A host error that says the extension may not read the URI at all (the
/// describe does not declare it) is a packaging fault, not a missing resource.
fn is_permission_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("permission") || m.contains("not declared") || m.contains("not permitted")
}

/// # Errors
/// A named error (always mentioning `vector-store/qdrant` when it concerns the
/// resource) when no usable connection exists.
pub fn resolve_connection<H: HostCalls>(host: &H, cfg: &Config) -> Result<Connection, RagError> {
    match host.secret(infra::URI) {
        Ok(doc) => from_infra(&doc),
        Err(msg) if is_permission_error(&msg) => Err(RagError::PermissionDenied(format!(
            "the host refused the {} resource: this extension's `permissions.secrets` must \
             declare {}",
            infra::KIND,
            infra::URI
        ))),
        Err(msg) => match infra::classify_host_error(&msg) {
            InfraError::Missing => legacy(host, cfg),
            other => Err(infra_error(other)),
        },
    }
}

fn from_infra(doc: &str) -> Result<Connection, RagError> {
    let conn = InfraConnection::parse(doc).map_err(infra_error)?;
    let dedicated = conn.extra(ISOLATION_KEY) == Some(DEDICATED);
    let prefix = match conn.require_extra(PREFIX_KEY) {
        Ok(p) => {
            validate_prefix(p)?;
            Some(p.to_string())
        }
        Err(_) if dedicated => None,
        Err(e) => return Err(infra_error(e)),
    };
    Ok(Connection {
        base_url: conn.base_url().to_string(),
        api_key: conn.credential().to_string(),
        collection_prefix: prefix,
        source: Source::Infra,
    })
}

fn legacy<H: HostCalls>(host: &H, cfg: &Config) -> Result<Connection, RagError> {
    if cfg.qdrant_url.trim().is_empty() {
        return Err(infra_error(InfraError::Missing));
    }
    let api_key = host.secret(LEGACY_KEY_REF).map_err(|e| {
        RagError::PermissionDenied(format!("host could not resolve {LEGACY_KEY_REF}: {e}"))
    })?;
    Ok(Connection {
        base_url: cfg.qdrant_url.clone(),
        api_key,
        collection_prefix: None,
        source: Source::Legacy,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChunkConfig, EmbeddingConfig};
    use crate::host::{HttpRequest, HttpResponse};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    const NOT_CONNECTED: &str = "resource vector-store/qdrant is not connected to this deployment";

    struct StubHost {
        secrets: HashMap<String, Result<String, String>>,
        asked: RefCell<Vec<String>>,
        fetches: Cell<usize>,
    }

    impl StubHost {
        fn new(entries: &[(&str, Result<&str, &str>)]) -> Self {
            Self {
                secrets: entries
                    .iter()
                    .map(|(k, v)| {
                        (
                            (*k).to_string(),
                            v.map(str::to_string).map_err(str::to_string),
                        )
                    })
                    .collect(),
                asked: RefCell::default(),
                fetches: Cell::new(0),
            }
        }
        fn asked(&self, uri: &str) -> bool {
            self.asked.borrow().iter().any(|a| a == uri)
        }
    }

    impl HostCalls for StubHost {
        fn fetch(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
            self.fetches.set(self.fetches.get() + 1);
            Err("no network in this test".into())
        }
        fn secret(&self, uri: &str) -> Result<String, String> {
            self.asked.borrow_mut().push(uri.to_string());
            self.secrets
                .get(uri)
                .cloned()
                .unwrap_or_else(|| Err(NOT_CONNECTED.to_string()))
        }
    }

    fn cfg(url: &str) -> Config {
        Config {
            qdrant_url: url.to_string(),
            collection: "kb".into(),
            embedding: EmbeddingConfig {
                base_url: "https://api.openai.com/v1".into(),
                model: "m".into(),
                dimensions: 3,
            },
            chunk: ChunkConfig {
                max_chars: 10,
                overlap_chars: 2,
            },
            require_tenant_overlay: false,
        }
    }

    fn doc(extras: &str) -> String {
        format!(
            r#"{{"v":1,"kind":"vector-store/qdrant","host":"q.example.com","port":6333,
            "scheme":"https","base_url":"https://q.example.com:6333","credential":"S3CRET",
            "extras":{extras}}}"#
        )
    }

    fn with_infra(extras: &str) -> StubHost {
        let d = doc(extras);
        StubHost::new(&[(infra::URI, Ok(&d)), (LEGACY_KEY_REF, Ok("legacy-key"))])
    }

    #[test]
    fn infra_present_wins_over_a_configured_legacy_url() {
        let host = with_infra(r#"{"collection_prefix":"acme__"}"#);
        let c = resolve_connection(&host, &cfg("https://old.qdrant.io:6333")).unwrap();
        assert_eq!(c.source, Source::Infra);
        assert_eq!(c.base_url, "https://q.example.com:6333");
        assert_eq!(c.api_key, "S3CRET");
        assert_eq!(c.collection_prefix.as_deref(), Some("acme__"));
        assert!(!host.asked(LEGACY_KEY_REF));
    }

    #[test]
    fn infra_missing_and_legacy_configured_uses_legacy() {
        let host = StubHost::new(&[(LEGACY_KEY_REF, Ok("legacy-key"))]);
        let c = resolve_connection(&host, &cfg("https://old.qdrant.io:6333")).unwrap();
        assert_eq!(c.source, Source::Legacy);
        assert_eq!(c.api_key, "legacy-key");
        assert_eq!(c.base_url, "https://old.qdrant.io:6333");
        assert_eq!(c.collection_prefix, None);
        assert!(host.asked(LEGACY_KEY_REF));
    }

    #[test]
    fn infra_missing_and_nothing_legacy_is_a_named_error_not_localhost() {
        let host = StubHost::new(&[]);
        let err = resolve_connection(&host, &cfg("")).unwrap_err().to_string();
        assert!(err.contains("vector-store/qdrant"), "{err}");
        assert!(err.contains("Add it in workspace settings"), "{err}");
        assert!(!err.contains("localhost") && !err.contains("127.0.0.1"));
        assert_eq!(host.fetches.get(), 0);
        assert!(!host.asked(LEGACY_KEY_REF));
    }

    #[test]
    fn infra_invalid_never_falls_back_to_legacy() {
        let bad = doc(r#"{}"#).replace("\"port\":6333", "\"port\":0");
        let host = StubHost::new(&[(infra::URI, Ok(&bad)), (LEGACY_KEY_REF, Ok("legacy-key"))]);
        let err = resolve_connection(&host, &cfg("https://old.qdrant.io:6333")).unwrap_err();
        assert!(err.to_string().contains("vector-store/qdrant"));
        assert!(!host.asked(LEGACY_KEY_REF));
    }

    #[test]
    fn infra_unreadable_never_falls_back_to_legacy() {
        let host = StubHost::new(&[
            (infra::URI, Err("backend timeout")),
            (LEGACY_KEY_REF, Ok("legacy-key")),
        ]);
        let err = resolve_connection(&host, &cfg("https://old.qdrant.io:6333")).unwrap_err();
        assert!(matches!(err, RagError::Internal(_)), "{err:?}");
        assert!(err.to_string().contains("vector-store/qdrant"));
        assert!(!host.asked(LEGACY_KEY_REF));
    }

    #[test]
    fn infra_unsupported_version_is_refused_by_name() {
        let d = doc("{}").replace("\"v\":1", "\"v\":2");
        let host = StubHost::new(&[(infra::URI, Ok(&d))]);
        let err = resolve_connection(&host, &cfg("https://old.qdrant.io:6333"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("version 2") && err.contains("vector-store/qdrant"),
            "{err}"
        );
    }

    #[test]
    fn a_missing_collection_prefix_fails_closed() {
        let host = with_infra("{}");
        let err = resolve_connection(&host, &cfg("")).unwrap_err().to_string();
        assert!(
            err.contains("collection_prefix") && err.contains("vector-store/qdrant"),
            "{err}"
        );

        let host = with_infra(r#"{"isolation":"dedicated"}"#);
        let c = resolve_connection(&host, &cfg("")).unwrap();
        assert_eq!(c.source, Source::Infra);
        assert_eq!(c.physical("kb").unwrap(), "kb");
    }

    #[test]
    fn a_blank_prefix_is_treated_as_missing_unless_dedicated() {
        let host = with_infra(r#"{"collection_prefix":"  "}"#);
        assert!(resolve_connection(&host, &cfg("")).is_err());
        let host = with_infra(r#"{"collection_prefix":"  ","isolation":"dedicated"}"#);
        assert_eq!(
            resolve_connection(&host, &cfg(""))
                .unwrap()
                .collection_prefix,
            None
        );
    }

    #[test]
    fn physical_collection_applies_the_prefix_once() {
        assert_eq!(
            physical_collection(Some("acme__"), "kb").unwrap(),
            "acme__kb"
        );
        assert_eq!(physical_collection(None, "kb").unwrap(), "kb");
    }

    #[test]
    fn physical_collection_refuses_names_that_could_escape_the_namespace() {
        for bad in ["", "a/b", "../x", "..", "__x", &"a".repeat(129)] {
            assert!(physical_collection(Some("acme__"), bad).is_err(), "{bad:?}");
            assert!(physical_collection(None, bad).is_err(), "{bad:?}");
        }
        // Not de-duplicated: that would be a cross-tenant hole.
        assert_eq!(
            physical_collection(Some("acme__"), "acme__kb").unwrap(),
            "acme__acme__kb"
        );
    }

    #[test]
    fn prefix_rules() {
        assert!(physical_collection(Some(&"p".repeat(65)), "kb").is_err());
        assert!(physical_collection(Some("a/b"), "kb").is_err());
        assert!(physical_collection(Some(""), "kb").is_err());
    }

    #[test]
    fn legacy_physical_names_are_untouched() {
        let host = StubHost::new(&[(LEGACY_KEY_REF, Ok("k"))]);
        let c = resolve_connection(&host, &cfg("https://old.qdrant.io:6333")).unwrap();
        assert_eq!(c.physical("anything goes").unwrap(), "anything goes");
    }

    #[test]
    fn an_undeclared_uri_is_a_packaging_error_naming_the_permission() {
        let host = StubHost::new(&[(infra::URI, Err("permission denied: undeclared secret"))]);
        let err = resolve_connection(&host, &cfg("https://old.qdrant.io:6333")).unwrap_err();
        assert!(matches!(err, RagError::PermissionDenied(_)));
        let s = err.to_string();
        assert!(
            s.contains("vector-store/qdrant") && s.contains("permissions.secrets"),
            "{s}"
        );
        assert!(!host.asked(LEGACY_KEY_REF));
    }

    #[test]
    fn the_credential_is_not_in_any_error_text() {
        let cases: Vec<StubHost> = vec![
            StubHost::new(&[]),
            StubHost::new(&[(infra::URI, Err("backend S3CRET"))]),
            with_infra("{}"),
            with_infra(r#"{"collection_prefix":"a/b"}"#),
            StubHost::new(&[(
                infra::URI,
                Ok(&doc("{}").replace("\"port\":6333", "\"port\":\"S3CRET\"")),
            )]),
        ];
        for host in cases {
            if let Err(e) = resolve_connection(&host, &cfg("")) {
                let shown = format!("{e} {e:?}");
                assert!(!shown.contains("S3CRET"), "{shown}");
            }
        }
        let host = with_infra(r#"{"collection_prefix":"acme__"}"#);
        let c = resolve_connection(&host, &cfg("")).unwrap();
        assert!(!format!("{c:?}").contains("S3CRET"));
    }
}
