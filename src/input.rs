//! Tool arguments: JSON in, validated typed values out. Pure.

use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use crate::error::RagError;

fn default_top_k() -> u32 {
    5
}

fn default_list_limit() -> u32 {
    50
}

fn default_distance() -> String {
    "Cosine".to_string()
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// The host's per-tenant configuration for this extension, delivered on every
/// call under the reserved args key `_tenant_overlay`.
///
/// This is the extension's *effective* config for the calling tenant — the
/// operator baseline deep-merged with that tenant's override — resolved by the
/// host from its own `extension_config` tables. It is emphatically **not**
/// caller input: both hosts strip `_tenant_overlay` from the caller's
/// arguments unconditionally and re-insert their own, including when a tenant
/// has no override configured. That unconditional strip is what makes this
/// trustworthy where the plain `collection` argument never can be — without
/// it, a caller could smuggle an overlay naming another tenant's collection
/// during the window when no override happened to be set.
///
/// Every field is optional, and unknown keys are ignored rather than rejected:
/// the blob's shape is this extension's to define, so a host that learns to
/// send more of it must not break a guest that has not learned to read it yet.
///
/// Deliberately **not** cached in a `static`. The process-wide config
/// `OnceLock` is per-instance; this is per-call, and one instance serves many
/// tenants.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TenantOverlay {
    /// The collection this tenant's data lives in. When present it is
    /// authoritative — see `ops::collection_of`.
    #[serde(default)]
    pub collection: Option<String>,
    /// Present in a fully-merged overlay because it is part of the baseline.
    /// This extension cannot honour a *differing* one — `qdrant_url` is read
    /// straight off the process config by every request builder — so a
    /// disagreement is refused rather than silently ignored.
    #[serde(default)]
    pub qdrant_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchInput {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    #[serde(default)]
    pub filter: Option<Value>,
    #[serde(default)]
    pub collection: Option<String>,
    /// Injected by the host, never by the caller. See [`TenantOverlay`].
    #[serde(rename = "_tenant_overlay", default)]
    pub tenant_overlay: Option<TenantOverlay>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpsertInput {
    pub id: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub vector: Option<Vec<f32>>,
    #[serde(default = "empty_object")]
    pub payload: Value,
    #[serde(default)]
    pub collection: Option<String>,
    /// Injected by the host, never by the caller. See [`TenantOverlay`].
    #[serde(rename = "_tenant_overlay", default)]
    pub tenant_overlay: Option<TenantOverlay>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IngestInput {
    pub doc_id: String,
    pub text: String,
    #[serde(default = "empty_object")]
    pub metadata: Value,
    #[serde(default)]
    pub collection: Option<String>,
    /// Injected by the host, never by the caller. See [`TenantOverlay`].
    #[serde(rename = "_tenant_overlay", default)]
    pub tenant_overlay: Option<TenantOverlay>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeleteInput {
    #[serde(default)]
    pub ids: Option<Vec<String>>,
    #[serde(default)]
    pub doc_id: Option<String>,
    #[serde(default)]
    pub collection: Option<String>,
    /// Injected by the host, never by the caller. See [`TenantOverlay`].
    #[serde(rename = "_tenant_overlay", default)]
    pub tenant_overlay: Option<TenantOverlay>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EnsureInput {
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub dimensions: Option<u32>,
    #[serde(default = "default_distance")]
    pub distance: String,
    /// Injected by the host, never by the caller. See [`TenantOverlay`].
    #[serde(rename = "_tenant_overlay", default)]
    pub tenant_overlay: Option<TenantOverlay>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ListInput {
    /// Page size: chunks scanned per Qdrant scroll call, not documents.
    /// Grouping by `doc_id` happens after the page comes back, so a doc whose
    /// chunks straddle a page boundary shows a partial count on each page.
    #[serde(default = "default_list_limit")]
    pub limit: u32,
    /// Opaque cursor from a previous call's `next_page_offset`. Omit to start
    /// from the first page.
    #[serde(default)]
    pub offset: Option<Value>,
    #[serde(default)]
    pub filter: Option<Value>,
    #[serde(default)]
    pub collection: Option<String>,
    /// Injected by the host, never by the caller. See [`TenantOverlay`].
    #[serde(rename = "_tenant_overlay", default)]
    pub tenant_overlay: Option<TenantOverlay>,
}

fn decode<T: for<'de> Deserialize<'de>>(tool: &str, json: &str) -> Result<T, RagError> {
    serde_json::from_str(json)
        .map_err(|e| RagError::InvalidInput(format!("decode {tool} input: {e}")))
}

/// Exactly one of a text field and a vector field must be present. Both means
/// the caller has two conflicting intents and we would have to silently pick
/// one; neither means there is nothing to search or store.
fn exactly_one(tool: &str, has_text: bool, has_vector: bool) -> Result<(), RagError> {
    match (has_text, has_vector) {
        (true, false) | (false, true) => Ok(()),
        (true, true) => Err(RagError::InvalidInput(format!(
            "{tool}: pass either a text field or `vector`, not both"
        ))),
        (false, false) => Err(RagError::InvalidInput(format!(
            "{tool}: pass either a text field or `vector`"
        ))),
    }
}

/// # Errors
/// [`RagError::InvalidInput`] on malformed JSON, a `top_k` of zero, or a
/// `query`/`vector` combination that is not exactly one.
pub fn parse_search(json: &str) -> Result<SearchInput, RagError> {
    let parsed: SearchInput = decode("rag_search", json)?;
    let has_query = parsed.query.as_ref().is_some_and(|q| !q.trim().is_empty());
    let has_vector = parsed.vector.as_ref().is_some_and(|v| !v.is_empty());
    exactly_one("rag_search", has_query, has_vector)?;
    if parsed.top_k == 0 {
        return Err(RagError::InvalidInput(
            "rag_search: top_k must be greater than zero".into(),
        ));
    }
    Ok(parsed)
}

/// Qdrant accepts only an unsigned integer or a UUID as a point id; anything
/// else is a 400 from Qdrant, not a value we should spend an embeddings call
/// and an ensure PUT on first.
fn validate_point_id(id: &str) -> Result<(), RagError> {
    if id.parse::<u64>().is_ok() || Uuid::parse_str(id).is_ok() {
        return Ok(());
    }
    Err(RagError::InvalidInput(format!(
        "rag_upsert: id {id:?} must be an unsigned integer or a UUID"
    )))
}

/// # Errors
/// [`RagError::InvalidInput`] on malformed JSON, an `id` that is not an
/// unsigned integer or a UUID, or a `text`/`vector` combination that is not
/// exactly one.
pub fn parse_upsert(json: &str) -> Result<UpsertInput, RagError> {
    let parsed: UpsertInput = decode("rag_upsert", json)?;
    if parsed.id.trim().is_empty() {
        return Err(RagError::InvalidInput("rag_upsert: id is empty".into()));
    }
    validate_point_id(&parsed.id)?;
    let has_text = parsed.text.as_ref().is_some_and(|t| !t.trim().is_empty());
    let has_vector = parsed.vector.as_ref().is_some_and(|v| !v.is_empty());
    exactly_one("rag_upsert", has_text, has_vector)?;
    Ok(parsed)
}

/// # Errors
/// [`RagError::InvalidInput`] on malformed JSON, an empty `doc_id`, or text
/// that is empty or whitespace-only.
pub fn parse_ingest(json: &str) -> Result<IngestInput, RagError> {
    let parsed: IngestInput = decode("rag_ingest", json)?;
    if parsed.doc_id.trim().is_empty() {
        return Err(RagError::InvalidInput("rag_ingest: doc_id is empty".into()));
    }
    if parsed.text.trim().is_empty() {
        return Err(RagError::InvalidInput("rag_ingest: text is empty".into()));
    }
    Ok(parsed)
}

/// # Errors
/// [`RagError::InvalidInput`] unless exactly one of `ids` (non-empty) and
/// `doc_id` is given. Accepting neither would delete everything.
pub fn parse_delete(json: &str) -> Result<DeleteInput, RagError> {
    let parsed: DeleteInput = decode("rag_delete", json)?;
    let has_ids = parsed.ids.as_ref().is_some_and(|i| !i.is_empty());
    let has_doc = parsed.doc_id.as_ref().is_some_and(|d| !d.trim().is_empty());
    match (has_ids, has_doc) {
        (true, false) | (false, true) => Ok(parsed),
        (true, true) => Err(RagError::InvalidInput(
            "rag_delete: pass either `ids` or `doc_id`, not both".into(),
        )),
        (false, false) => Err(RagError::InvalidInput(
            "rag_delete: pass either `ids` or `doc_id`".into(),
        )),
    }
}

/// # Errors
/// [`RagError::InvalidInput`] on malformed JSON or a distance metric Qdrant
/// does not implement.
pub fn parse_ensure(json: &str) -> Result<EnsureInput, RagError> {
    let parsed: EnsureInput = decode("rag_collection_ensure", json)?;
    // Qdrant also implements "Manhattan", left out here because it is a poor
    // fit for normalised embeddings and degrades recall silently rather than
    // failing. Adding it later is a one-line change.
    const ALLOWED: [&str; 3] = ["Cosine", "Dot", "Euclid"];
    if !ALLOWED.contains(&parsed.distance.as_str()) {
        return Err(RagError::InvalidInput(format!(
            "rag_collection_ensure: distance must be one of {ALLOWED:?}, got {:?}",
            parsed.distance
        )));
    }
    Ok(parsed)
}

/// # Errors
/// [`RagError::InvalidInput`] on malformed JSON or a `limit` of zero.
pub fn parse_list(json: &str) -> Result<ListInput, RagError> {
    let parsed: ListInput = decode("rag_list", json)?;
    if parsed.limit == 0 {
        return Err(RagError::InvalidInput(
            "rag_list: limit must be greater than zero".into(),
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_accepts_a_text_query() {
        let got = parse_search(r#"{"query":"kemayoran units","top_k":3}"#).unwrap();
        assert_eq!(got.query.as_deref(), Some("kemayoran units"));
        assert_eq!(got.top_k, 3);
        assert!(got.vector.is_none());
    }

    #[test]
    fn search_accepts_a_raw_vector() {
        let got = parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap();
        assert_eq!(got.vector.as_deref(), Some([0.1f32, 0.2, 0.3].as_slice()));
        assert!(got.query.is_none());
    }

    #[test]
    fn search_top_k_defaults_to_five() {
        assert_eq!(parse_search(r#"{"query":"x"}"#).unwrap().top_k, 5);
    }

    #[test]
    fn search_rejects_both_query_and_vector() {
        let err = parse_search(r#"{"query":"x","vector":[0.1]}"#).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)));
    }

    #[test]
    fn search_rejects_neither_query_nor_vector() {
        let err = parse_search(r#"{"top_k":3}"#).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)));
    }

    #[test]
    fn search_rejects_a_zero_top_k() {
        assert!(parse_search(r#"{"query":"x","top_k":0}"#).is_err());
    }

    #[test]
    fn upsert_enforces_the_same_query_vector_rule() {
        assert!(parse_upsert(r#"{"id":"1","text":"hi"}"#).is_ok());
        assert!(parse_upsert(r#"{"id":"1","vector":[0.1]}"#).is_ok());
        assert!(parse_upsert(r#"{"id":"1","text":"hi","vector":[0.1]}"#).is_err());
        assert!(parse_upsert(r#"{"id":"1"}"#).is_err());
    }

    #[test]
    fn upsert_requires_a_non_empty_id() {
        assert!(parse_upsert(r#"{"id":"","text":"hi"}"#).is_err());
    }

    #[test]
    fn upsert_accepts_a_numeric_id() {
        assert!(parse_upsert(r#"{"id":"42","text":"hi"}"#).is_ok());
    }

    #[test]
    fn upsert_accepts_a_canonical_uuid() {
        let json = r#"{"id":"3f2504e0-4f89-41d3-9a0c-0305e82c3301","text":"hi"}"#;
        assert!(parse_upsert(json).is_ok());
    }

    #[test]
    fn upsert_rejects_an_id_that_is_neither_an_integer_nor_a_uuid() {
        let err = parse_upsert(r#"{"id":"policy-2024","text":"hi"}"#).unwrap_err();
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput");
        };
        assert!(msg.contains("policy-2024"), "message was: {msg}");
    }

    #[test]
    fn upsert_rejects_an_empty_id_before_the_id_shape_check() {
        let err = parse_upsert(r#"{"id":"","text":"hi"}"#).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)));
    }

    #[test]
    fn ingest_requires_a_doc_id_and_text() {
        let got = parse_ingest(r#"{"doc_id":"d1","text":"body"}"#).unwrap();
        assert_eq!(got.doc_id, "d1");
        assert_eq!(got.metadata, serde_json::json!({}));
        assert!(parse_ingest(r#"{"text":"body"}"#).is_err());
        assert!(parse_ingest(r#"{"doc_id":"d1","text":"   "}"#).is_err());
    }

    #[test]
    fn delete_requires_exactly_one_selector() {
        assert!(parse_delete(r#"{"ids":["a"]}"#).is_ok());
        assert!(parse_delete(r#"{"doc_id":"d1"}"#).is_ok());
        assert!(parse_delete(r#"{"ids":["a"],"doc_id":"d1"}"#).is_err());
        assert!(parse_delete(r#"{}"#).is_err());
        assert!(parse_delete(r#"{"ids":[]}"#).is_err());
    }

    #[test]
    fn ensure_defaults_distance_to_cosine_and_rejects_unknown_metrics() {
        assert_eq!(parse_ensure(r#"{}"#).unwrap().distance, "Cosine");
        assert_eq!(
            parse_ensure(r#"{"distance":"Dot"}"#).unwrap().distance,
            "Dot"
        );
        assert!(parse_ensure(r#"{"distance":"Manhattan"}"#).is_err());
    }

    #[test]
    fn list_defaults_limit_and_leaves_offset_and_filter_unset() {
        let got = parse_list(r#"{}"#).unwrap();
        assert_eq!(got.limit, 50);
        assert!(got.offset.is_none());
        assert!(got.filter.is_none());
        assert!(got.collection.is_none());
    }

    #[test]
    fn list_rejects_a_zero_limit() {
        let err = parse_list(r#"{"limit":0}"#).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)));
    }

    #[test]
    fn list_accepts_an_explicit_offset_and_filter() {
        let got = parse_list(r#"{"limit":10,"offset":"abc","filter":{"must":[]}}"#).unwrap();
        assert_eq!(got.limit, 10);
        assert_eq!(got.offset, Some(serde_json::json!("abc")));
        assert_eq!(got.filter, Some(serde_json::json!({"must":[]})));
    }
}
