//! Typed reader for the connection document the host stages for the
//! `vector-store/qdrant` infrastructure resource.
//!
//! The document is read through `host.secrets.get(URI)` and is JSON:
//! `{"v":1,"kind","host","port","scheme","base_url","credential","extras"}`.
//! `tests/fixtures/infra_connection_v1.json` is the golden copy of that
//! document (source: the runtime plan's guest-document fixture; if the
//! runtime changes the shape, re-sync the fixture and this reader together).
//! Local until the extension SDK ships an equivalent helper.
//!
//! Pure module: no WIT. Nothing here ever formats the credential or the raw
//! document; parse failures are mapped to fixed strings because serde errors
//! echo the offending value.

use std::collections::BTreeMap;

use serde_json::Value;

/// The resource kind this extension needs.
pub const KIND: &str = "vector-store/qdrant";
/// The canonical guest URI of the kind. Must be declared verbatim in
/// `runtime.permissions.secrets`.
pub const URI: &str = "secret://infra/vector_store_qdrant";
/// The kind as the runner writes it into the guest document: the canonical
/// (secret-name) form of [`KIND`]. The URI already selects the kind, so this is
/// a consistency check, not a selector. Source of truth: the runner's
/// `to_guest_document` and its `infra_connection_v1.json` fixture.
pub const DOCUMENT_KIND: &str = "vector_store_qdrant";
/// Highest document version this reader understands.
pub const SUPPORTED_VERSION: u32 = 1;

/// The host's wording when the resource is not connected to the deployment.
/// The wordings that mean "this resource is simply not connected here", taken
/// from the runner source.
///
/// - `greentic-aw-runtime` `infra_connection.rs` (deployed workers):
///   "infra resource '<kind>' is not connected to this deployment (...)".
/// - Designer in-process hosts: `SecretsError::NotFound` displayed as
///   "secret not found: secret://infra/...".
/// - `greentic-runner-host` `agent_node.rs`, out-of-process serve path: "infra
///   resource request '<uri>' is not available on the out-of-process serve path".
///
/// Anything else (bridge down, permission, backend) is NOT missing and fails
/// closed.
const NOT_CONNECTED_PHRASE: &str = "is not connected to this deployment";
const NOT_FOUND_PHRASE: &str = "secret not found: secret://infra/";
const SERVE_PATH_PHRASE: &str = "not available on the out-of-process serve path";

pub struct InfraConnection {
    base_url: String,
    // Kept for diagnostics (never displayed with the credential).
    #[allow(dead_code)]
    host: String,
    #[allow(dead_code)]
    port: u16,
    #[allow(dead_code)]
    scheme: String,
    credential: String,
    extras: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfraError {
    /// The host said the resource is not connected / not declared.
    Missing,
    /// The host failed in some other way.
    Unreadable,
    /// The document is malformed; `field` names which part, never its value.
    Invalid {
        field: &'static str,
    },
    UnsupportedVersion {
        found: u32,
    },
    MissingExtra {
        key: String,
    },
}

impl core::fmt::Display for InfraError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Missing => write!(
                f,
                "no {KIND} resource is connected to this deployment. Add it in workspace \
                 settings (Resources), then redeploy"
            ),
            Self::Unreadable => write!(f, "the {KIND} resource could not be read from the host"),
            Self::Invalid { field } => write!(
                f,
                "the {KIND} resource document is invalid (field `{field}`)"
            ),
            Self::UnsupportedVersion { found } => write!(
                f,
                "the {KIND} resource document is version {found}, but this extension \
                 understands up to version {SUPPORTED_VERSION}; update the extension"
            ),
            Self::MissingExtra { key } => write!(
                f,
                "the {KIND} resource has no `{key}` setting, which this extension requires"
            ),
        }
    }
}

impl core::fmt::Debug for InfraConnection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InfraConnection")
            .field("base_url", &self.base_url)
            .field("credential", &"<redacted>")
            .field("extras", &self.extras.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn str_field(doc: &Value, field: &'static str) -> Result<String, InfraError> {
    match doc.get(field) {
        Some(Value::String(s)) if !s.trim().is_empty() => Ok(s.clone()),
        _ => Err(InfraError::Invalid { field }),
    }
}

/// `base_url` must be `http(s)://authority` only: no userinfo, path, query or
/// fragment, because request URLs are built by appending `/collections/...`.
fn validate_base_url(base_url: &str) -> Result<(), InfraError> {
    let invalid = InfraError::Invalid { field: "base_url" };
    let Some((scheme, rest)) = base_url.split_once("://") else {
        return Err(invalid);
    };
    if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
        return Err(invalid);
    }
    if rest.is_empty()
        || rest.contains(['/', '?', '#', '@'])
        || rest.chars().any(char::is_whitespace)
    {
        return Err(invalid);
    }
    Ok(())
}

impl InfraConnection {
    /// # Errors
    /// [`InfraError::Invalid`] / [`InfraError::UnsupportedVersion`]; the error
    /// never carries a value from the document.
    pub fn parse(bytes: &str) -> Result<Self, InfraError> {
        let doc: Value =
            serde_json::from_str(bytes).map_err(|_| InfraError::Invalid { field: "document" })?;
        if !doc.is_object() {
            return Err(InfraError::Invalid { field: "document" });
        }

        let version = match doc.get("v") {
            None => u64::from(SUPPORTED_VERSION),
            Some(v) => v.as_u64().ok_or(InfraError::Invalid { field: "v" })?,
        };
        if version > u64::from(SUPPORTED_VERSION) {
            return Err(InfraError::UnsupportedVersion {
                found: u32::try_from(version).unwrap_or(u32::MAX),
            });
        }
        if version == 0 {
            return Err(InfraError::Invalid { field: "v" });
        }

        if let Some(kind) = doc.get("kind")
            && kind.as_str() != Some(DOCUMENT_KIND)
        {
            return Err(InfraError::Invalid { field: "kind" });
        }

        let host = str_field(&doc, "host")?;
        let scheme = str_field(&doc, "scheme")?;
        let base_url = str_field(&doc, "base_url")?
            .trim()
            .trim_end_matches('/')
            .to_string();
        validate_base_url(&base_url)?;
        let credential = str_field(&doc, "credential")?;
        let port = doc
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p != 0)
            .ok_or(InfraError::Invalid { field: "port" })?;

        let mut extras = BTreeMap::new();
        match doc.get("extras") {
            None | Some(Value::Null) => {}
            Some(Value::Object(map)) => {
                for (k, v) in map {
                    let Value::String(s) = v else {
                        return Err(InfraError::Invalid { field: "extras" });
                    };
                    extras.insert(k.clone(), s.clone());
                }
            }
            Some(_) => return Err(InfraError::Invalid { field: "extras" }),
        }

        Ok(Self {
            base_url,
            host,
            port,
            scheme,
            credential,
            extras,
        })
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Only caller: the request header builder.
    #[must_use]
    pub fn credential(&self) -> &str {
        &self.credential
    }

    #[must_use]
    pub fn extra(&self, key: &str) -> Option<&str> {
        self.extras.get(key).map(String::as_str)
    }

    /// A blank value counts as absent.
    ///
    /// # Errors
    /// [`InfraError::MissingExtra`].
    pub fn require_extra(&self, key: &str) -> Result<&str, InfraError> {
        self.extra(key)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| InfraError::MissingExtra { key: key.into() })
    }
}

/// Classify the host's error string for the guest-secret read. The host's
/// "not connected" wording means the resource is simply absent; anything else
/// is an unreadable resource, which must never fall back to another source.
#[must_use]
pub fn classify_host_error(msg: &str) -> InfraError {
    if [NOT_CONNECTED_PHRASE, NOT_FOUND_PHRASE, SERVE_PATH_PHRASE]
        .iter()
        .any(|p| msg.contains(p))
    {
        InfraError::Missing
    } else {
        InfraError::Unreadable
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Copied verbatim from the runner (greentic-aw-runtime
    /// `tests/fixtures/infra_connection_v1.json`); re-sync when it changes.
    const GOLDEN: &str = include_str!("../tests/fixtures/infra_connection_v1.json");

    fn golden_with(f: impl FnOnce(&mut Value)) -> String {
        let mut v: Value = serde_json::from_str(GOLDEN).unwrap();
        f(&mut v);
        v.to_string()
    }

    #[test]
    fn parses_the_golden_guest_document() {
        let c = InfraConnection::parse(GOLDEN).unwrap();
        assert_eq!(c.base_url(), "https://q.example.com:6333");
        assert_eq!(c.credential(), "S3CRET");
        assert_eq!(c.extra("collection_prefix"), Some("acme__"));
    }

    #[test]
    fn base_url_must_be_http_or_https_authority_only() {
        for bad in [
            "ftp://q.example.com:6333",
            "q.example.com:6333",
            "https://q.example.com:6333/x",
            "https://q.example.com:6333/?y=1",
            "https://q.example.com:6333?y=1",
            "https://user:pw@q.example.com:6333",
            "https://q.example.com:6333#f",
            "https://",
        ] {
            let doc = golden_with(|v| v["base_url"] = bad.into());
            assert_eq!(
                InfraConnection::parse(&doc).unwrap_err(),
                InfraError::Invalid { field: "base_url" },
                "{bad}"
            );
        }
        for good in ["http://q.example.com:6333", "https://q.example.com:6333/"] {
            let doc = golden_with(|v| v["base_url"] = good.into());
            assert!(InfraConnection::parse(&doc).is_ok(), "{good}");
        }
    }

    #[test]
    fn the_runners_fixture_kind_is_the_canonical_name() {
        // The runner sends the canonical secret name, not `vector-store/qdrant`.
        let v: Value = serde_json::from_str(GOLDEN).unwrap();
        assert_eq!(v["kind"], DOCUMENT_KIND);
        assert!(InfraConnection::parse(GOLDEN).is_ok());
    }

    #[test]
    fn an_absent_version_is_read_as_one_and_a_higher_one_is_refused() {
        let no_v = golden_with(|v| {
            v.as_object_mut().unwrap().remove("v");
        });
        assert!(InfraConnection::parse(&no_v).is_ok());
        let v2 = golden_with(|v| v["v"] = 2.into());
        assert_eq!(
            InfraConnection::parse(&v2).unwrap_err(),
            InfraError::UnsupportedVersion { found: 2 }
        );
    }

    #[test]
    fn base_url_is_taken_from_the_host_document_not_rebuilt() {
        let doc = golden_with(|v| {
            v["host"] = "::1".into();
            v["scheme"] = "http".into();
            v["base_url"] = "http://[::1]:6333".into();
        });
        assert_eq!(
            InfraConnection::parse(&doc).unwrap().base_url(),
            "http://[::1]:6333"
        );
    }

    #[test]
    fn a_trailing_slash_on_base_url_is_trimmed() {
        let doc = golden_with(|v| v["base_url"] = "https://q.example.com:6333/".into());
        assert_eq!(
            InfraConnection::parse(&doc).unwrap().base_url(),
            "https://q.example.com:6333"
        );
    }

    #[test]
    fn invalid_documents_name_the_field_and_never_echo_a_value() {
        let cases: Vec<(String, &str)> = vec![
            (golden_with(|v| v["port"] = "S3CRET-PORT".into()), "port"),
            (golden_with(|v| v["port"] = 0.into()), "port"),
            (golden_with(|v| v["credential"] = "".into()), "credential"),
            (
                golden_with(|v| {
                    v.as_object_mut().unwrap().remove("base_url");
                }),
                "base_url",
            ),
            (
                golden_with(|v| v["extras"] = serde_json::json!({"S3CRET": 1})),
                "extras",
            ),
            ("not json S3CRET".to_string(), "document"),
            ("[\"S3CRET\"]".to_string(), "document"),
            (golden_with(|v| v["kind"] = "other_kind".into()), "kind"),
            // The display form of the kind is not what the runner sends.
            (
                golden_with(|v| v["kind"] = "vector-store/qdrant".into()),
                "kind",
            ),
        ];
        for (doc, field) in cases {
            let err = InfraConnection::parse(&doc).unwrap_err();
            assert_eq!(err, InfraError::Invalid { field }, "doc: {doc}");
            assert!(!err.to_string().contains("S3CRET"));
            assert!(!format!("{err:?}").contains("S3CRET"));
        }
    }

    #[test]
    fn debug_never_shows_the_credential() {
        let c = InfraConnection::parse(GOLDEN).unwrap();
        assert!(!format!("{c:?}").contains("S3CRET"));
        assert!(format!("{c:?}").contains("<redacted>"));
    }

    #[test]
    fn require_extra_treats_blank_as_absent() {
        let doc = golden_with(|v| v["extras"]["collection_prefix"] = "  ".into());
        let c = InfraConnection::parse(&doc).unwrap();
        assert_eq!(
            c.require_extra("collection_prefix").unwrap_err(),
            InfraError::MissingExtra {
                key: "collection_prefix".into()
            }
        );
        assert_eq!(
            InfraConnection::parse(GOLDEN)
                .unwrap()
                .require_extra("collection_prefix"),
            Ok("acme__")
        );
    }

    use crate::testkit::{REAL_NOT_CONNECTED, REAL_NOT_FOUND, REAL_SERVE_PATH};

    #[test]
    fn the_deployed_worker_wording_is_missing() {
        assert_eq!(classify_host_error(REAL_NOT_CONNECTED), InfraError::Missing);
    }

    #[test]
    fn the_designer_in_process_wording_is_missing() {
        assert_eq!(classify_host_error(REAL_NOT_FOUND), InfraError::Missing);
    }

    #[test]
    fn the_out_of_process_serve_path_wording_is_missing() {
        assert_eq!(classify_host_error(REAL_SERVE_PATH), InfraError::Missing);
    }

    #[test]
    fn everything_else_fails_closed_as_unreadable() {
        for msg in [
            "backend timeout",
            "bridge unavailable",
            "secret not found: secret://rag-qdrant/qdrant_api_key",
            "permission denied",
        ] {
            assert_eq!(classify_host_error(msg), InfraError::Unreadable, "{msg}");
        }
    }

    #[test]
    fn display_names_the_kind() {
        for e in [
            InfraError::Missing,
            InfraError::Unreadable,
            InfraError::Invalid { field: "port" },
            InfraError::UnsupportedVersion { found: 9 },
            InfraError::MissingExtra { key: "k".into() },
        ] {
            assert!(e.to_string().contains("vector-store/qdrant"), "{e}");
        }
    }
}
