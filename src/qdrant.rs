//! Qdrant REST request builders and response parsers. Pure.

use serde_json::Value;
use uuid::Uuid;

use crate::error::RagError;
use crate::host::HttpRequest;

/// Fixed namespace for chunk point ids. Any stable UUID works; this one is the
/// RFC 4122 "X.500" namespace, chosen only because it is a constant nobody else
/// in this extension will reuse. Changing it orphans every existing point.
const CHUNK_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x12, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub id: String,
    pub vector: Vec<f32>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub id: String,
    pub score: f32,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeleteSelector {
    Ids(Vec<String>),
    DocId(String),
}

/// Deterministic UUIDv5 point id for one chunk of one document.
///
/// Determinism is what makes re-ingestion an overwrite instead of a duplicate.
#[must_use]
pub fn chunk_point_id(doc_id: &str, chunk_index: usize) -> String {
    Uuid::new_v5(&CHUNK_NAMESPACE, format!("{doc_id}:{chunk_index}").as_bytes()).to_string()
}

fn request(method: &str, url: String, api_key: &str, body: &Value) -> HttpRequest {
    HttpRequest {
        method: method.to_string(),
        url,
        headers: vec![
            ("api-key".to_string(), api_key.to_string()),
            ("content-type".to_string(), "application/json".to_string()),
        ],
        body: Some(serde_json::to_vec(body).unwrap_or_default()),
    }
}

#[must_use]
pub fn ensure_collection_request(
    base: &str,
    collection: &str,
    dimensions: u32,
    distance: &str,
    api_key: &str,
) -> HttpRequest {
    let body = serde_json::json!({ "vectors": { "size": dimensions, "distance": distance } });
    request("PUT", format!("{base}/collections/{collection}"), api_key, &body)
}

/// `wait=true` because a search issued immediately after an ingest would
/// otherwise race the index and return nothing.
#[must_use]
pub fn upsert_request(base: &str, collection: &str, points: &[Point], api_key: &str) -> HttpRequest {
    let points: Vec<Value> = points
        .iter()
        .map(|p| serde_json::json!({ "id": p.id, "vector": p.vector, "payload": p.payload }))
        .collect();
    let body = serde_json::json!({ "points": points });
    request(
        "PUT",
        format!("{base}/collections/{collection}/points?wait=true"),
        api_key,
        &body,
    )
}

#[must_use]
pub fn query_request(
    base: &str,
    collection: &str,
    vector: &[f32],
    top_k: u32,
    filter: Option<&Value>,
    api_key: &str,
) -> HttpRequest {
    let mut body = serde_json::json!({
        "query": vector,
        "limit": top_k,
        "with_payload": true,
    });
    if let (Some(filter), Some(map)) = (filter, body.as_object_mut()) {
        map.insert("filter".to_string(), filter.clone());
    }
    request(
        "POST",
        format!("{base}/collections/{collection}/points/query"),
        api_key,
        &body,
    )
}

#[must_use]
pub fn delete_request(
    base: &str,
    collection: &str,
    selector: &DeleteSelector,
    api_key: &str,
) -> HttpRequest {
    let body = match selector {
        DeleteSelector::Ids(ids) => serde_json::json!({ "points": ids }),
        DeleteSelector::DocId(doc_id) => serde_json::json!({
            "filter": { "must": [ { "key": "doc_id", "match": { "value": doc_id } } ] }
        }),
    };
    request(
        "POST",
        format!("{base}/collections/{collection}/points/delete?wait=true"),
        api_key,
        &body,
    )
}

/// Map a Qdrant HTTP status onto the extension's error taxonomy.
fn status_error(status: u16, body: &[u8]) -> Option<RagError> {
    let text = String::from_utf8_lossy(body).into_owned();
    match status {
        200..=299 => None,
        401 | 403 => Some(RagError::PermissionDenied(format!(
            "Qdrant rejected the api-key (HTTP {status}): {text}"
        ))),
        404 => Some(RagError::NotFound(format!(
            "Qdrant returned 404 — collection or point missing: {text}"
        ))),
        _ => Some(RagError::Internal(format!(
            "Qdrant returned HTTP {status}: {text}"
        ))),
    }
}

/// Parse the Query API envelope `{"result":{"points":[...]}}`.
///
/// # Errors
/// See [`status_error`]; a 2xx body that is not the expected shape is
/// [`RagError::Internal`].
pub fn parse_hits(status: u16, body: &[u8]) -> Result<Vec<Hit>, RagError> {
    if let Some(err) = status_error(status, body) {
        return Err(err);
    }
    let parsed: Value = serde_json::from_slice(body)
        .map_err(|e| RagError::Internal(format!("Qdrant response is not JSON: {e}")))?;

    let points = parsed
        .get("result")
        .and_then(|r| r.get("points"))
        .and_then(Value::as_array)
        .ok_or_else(|| RagError::Internal("Qdrant response has no result.points".to_string()))?;

    Ok(points
        .iter()
        .map(|point| Hit {
            // Qdrant ids are integers or UUIDs; normalise both to a string so
            // callers get one type.
            id: match point.get("id") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => String::new(),
            },
            score: point
                .get("score")
                .and_then(Value::as_f64)
                .unwrap_or_default() as f32,
            payload: point.get("payload").cloned().unwrap_or(Value::Null),
        })
        .collect())
}

/// Accept any 2xx for write operations.
///
/// # Errors
/// See [`status_error`].
pub fn parse_ack(status: u16, body: &[u8]) -> Result<(), RagError> {
    status_error(status, body).map_or(Ok(()), Err)
}

/// Like [`parse_ack`], but an "already exists" rejection is success.
///
/// Ensure runs on the ingest and upsert paths, so the collection existing is
/// the normal case rather than a fault.
///
/// # Errors
/// See [`status_error`], minus the already-exists case.
pub fn parse_ensure_ack(status: u16, body: &[u8]) -> Result<(), RagError> {
    let text = String::from_utf8_lossy(body).to_ascii_lowercase();
    if status == 409 || text.contains("already exists") {
        return Ok(());
    }
    parse_ack(status, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::RagError;
    use crate::host::HttpRequest;

    const BASE: &str = "https://c.qdrant.io:6333";

    fn body_of(req: &HttpRequest) -> serde_json::Value {
        serde_json::from_slice(req.body.as_deref().unwrap()).unwrap()
    }

    #[test]
    fn every_request_carries_the_api_key_header() {
        let reqs = [
            ensure_collection_request(BASE, "kb", 8, "Cosine", "k"),
            upsert_request(BASE, "kb", &[], "k"),
            query_request(BASE, "kb", &[0.1], 5, None, "k"),
            delete_request(BASE, "kb", &DeleteSelector::DocId("d".into()), "k"),
        ];
        for req in &reqs {
            assert!(
                req.headers.contains(&("api-key".to_string(), "k".to_string())),
                "missing api-key on {}",
                req.url
            );
        }
    }

    #[test]
    fn ensure_collection_puts_the_vector_params() {
        let req = ensure_collection_request(BASE, "kb", 1536, "Cosine", "k");
        assert_eq!(req.method, "PUT");
        assert_eq!(req.url, "https://c.qdrant.io:6333/collections/kb");
        assert_eq!(
            body_of(&req),
            serde_json::json!({"vectors":{"size":1536,"distance":"Cosine"}})
        );
    }

    #[test]
    fn upsert_waits_for_the_write_to_be_visible() {
        // Without wait=true a search issued straight after an ingest can miss
        // the points that ingest just wrote.
        let point = Point {
            id: "3f2504e0-4f89-41d3-9a0c-0305e82c3301".to_string(),
            vector: vec![0.1, 0.2],
            payload: serde_json::json!({"doc_id":"d1","chunk_index":0}),
        };
        let req = upsert_request(BASE, "kb", &[point], "k");
        assert_eq!(req.method, "PUT");
        assert_eq!(
            req.url,
            "https://c.qdrant.io:6333/collections/kb/points?wait=true"
        );
        let body = body_of(&req);
        assert_eq!(body["points"][0]["id"], "3f2504e0-4f89-41d3-9a0c-0305e82c3301");
        assert_eq!(body["points"][0]["payload"]["doc_id"], "d1");
    }

    #[test]
    fn query_uses_the_query_api_not_the_legacy_search_path() {
        let req = query_request(BASE, "kb", &[0.1, 0.2], 7, None, "k");
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.url,
            "https://c.qdrant.io:6333/collections/kb/points/query"
        );
        assert!(!req.url.contains("/points/search"));
        let body = body_of(&req);
        assert_eq!(body["limit"], 7);
        assert_eq!(body["with_payload"], true);
        // f32 precision: convert expected values to f32 and back to match serialized form
        assert_eq!(body["query"], serde_json::json!([0.1f32 as f64, 0.2f32 as f64]));
        assert!(body.get("filter").is_none());
    }

    #[test]
    fn a_query_filter_is_passed_through_verbatim() {
        let filter = serde_json::json!({"must":[{"key":"lang","match":{"value":"id"}}]});
        let req = query_request(BASE, "kb", &[0.1], 3, Some(&filter), "k");
        assert_eq!(body_of(&req)["filter"], filter);
    }

    #[test]
    fn delete_by_ids_sends_a_points_list() {
        let sel = DeleteSelector::Ids(vec!["a".into(), "b".into()]);
        let req = delete_request(BASE, "kb", &sel, "k");
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.url,
            "https://c.qdrant.io:6333/collections/kb/points/delete?wait=true"
        );
        assert_eq!(body_of(&req)["points"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn delete_by_doc_id_sends_a_payload_filter() {
        let sel = DeleteSelector::DocId("d1".into());
        let req = delete_request(BASE, "kb", &sel, "k");
        assert_eq!(
            body_of(&req)["filter"],
            serde_json::json!({"must":[{"key":"doc_id","match":{"value":"d1"}}]})
        );
    }

    #[test]
    fn chunk_ids_are_stable_uuids_that_differ_per_chunk_and_per_doc() {
        let a = chunk_point_id("doc-1", 0);
        assert_eq!(a, chunk_point_id("doc-1", 0), "must be deterministic");
        assert_ne!(a, chunk_point_id("doc-1", 1));
        assert_ne!(a, chunk_point_id("doc-2", 0));
        // Qdrant only accepts an unsigned integer or a UUID.
        assert_eq!(a.len(), 36);
        assert_eq!(a.matches('-').count(), 4);
    }

    #[test]
    fn hits_are_parsed_from_the_query_api_envelope() {
        let body = br#"{"result":{"points":[
            {"id":"p1","score":0.91,"payload":{"text":"hello"}},
            {"id":"p2","score":0.42,"payload":{"text":"world"}}
        ]},"status":"ok"}"#;
        let hits = parse_hits(200, body).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, "p1");
        assert!((hits[0].score - 0.91).abs() < 1e-6);
        assert_eq!(hits[1].payload["text"], "world");
    }

    #[test]
    fn a_numeric_point_id_is_rendered_as_a_string() {
        let body = br#"{"result":{"points":[{"id":42,"score":0.5,"payload":{}}]}}"#;
        assert_eq!(parse_hits(200, body).unwrap()[0].id, "42");
    }

    #[test]
    fn status_codes_map_onto_distinct_errors() {
        assert!(matches!(
            parse_hits(401, b"nope").unwrap_err(),
            RagError::PermissionDenied(_)
        ));
        assert!(matches!(
            parse_hits(404, b"missing").unwrap_err(),
            RagError::NotFound(_)
        ));
        assert!(matches!(
            parse_hits(503, b"down").unwrap_err(),
            RagError::Internal(_)
        ));
        assert!(matches!(
            parse_hits(200, b"<html>").unwrap_err(),
            RagError::Internal(_)
        ));
    }

    #[test]
    fn an_ack_only_needs_a_2xx() {
        assert!(parse_ack(200, br#"{"status":"ok"}"#).is_ok());
        assert!(parse_ack(404, b"gone").is_err());
    }

    #[test]
    fn creating_a_collection_that_already_exists_counts_as_success() {
        // Ensure is called on every ingest, so "already exists" is the normal
        // case, not an error.
        assert!(parse_ensure_ack(409, b"already exists").is_ok());
        assert!(
            parse_ensure_ack(400, br#"{"status":{"error":"Collection `kb` already exists!"}}"#)
                .is_ok()
        );
        assert!(parse_ensure_ack(400, br#"{"status":{"error":"bad dim"}}"#).is_err());
    }
}
