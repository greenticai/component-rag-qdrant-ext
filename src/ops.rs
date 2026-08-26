//! Orchestration: the only place in the extension that issues more than one
//! host call per tool. Generic over `HostCalls` so every sequence is testable.

use serde_json::Value;

use crate::chunk::chunk_text;
use crate::config::Config;
use crate::embed::{embed_request, parse_embed_response};
use crate::error::RagError;
use crate::host::{HostCalls, HttpRequest, HttpResponse};
use crate::input::{DeleteInput, EnsureInput, IngestInput, SearchInput, UpsertInput};
use crate::qdrant::{
    DeleteSelector, Point, chunk_point_id, delete_request, ensure_collection_request, parse_ack,
    parse_ensure_ack, parse_hits, query_request, upsert_request,
};

/// Secret URIs. These must match `runtime.permissions.secrets` in describe.json
/// verbatim — the host matches on the exact string (or a `/`-boundary prefix).
pub const QDRANT_KEY_REF: &str = "secret://rag-qdrant/qdrant_api_key";
pub const EMBEDDING_KEY_REF: &str = "secret://rag-qdrant/embedding_api_key";

fn send<H: HostCalls>(host: &H, req: &HttpRequest) -> Result<HttpResponse, RagError> {
    host.fetch(req)
        .map_err(|e| RagError::Internal(format!("host http::fetch failed: {e}")))
}

fn secret<H: HostCalls>(host: &H, uri: &str) -> Result<String, RagError> {
    host.secret(uri).map_err(|e| {
        RagError::PermissionDenied(format!("host could not resolve {uri}: {e}"))
    })
}

fn collection_of<'a>(cfg: &'a Config, override_: Option<&'a String>) -> &'a str {
    override_.map_or(cfg.collection.as_str(), String::as_str)
}

/// Embed a batch of texts in one call.
fn embed_all<H: HostCalls>(
    host: &H,
    cfg: &Config,
    texts: &[String],
) -> Result<Vec<Vec<f32>>, RagError> {
    let key = secret(host, EMBEDDING_KEY_REF)?;
    let req = embed_request(&cfg.embedding, texts, &key);
    let resp = send(host, &req)?;
    let vectors = parse_embed_response(resp.status, &resp.body, cfg.embedding.dimensions)?;
    if vectors.len() != texts.len() {
        return Err(RagError::Internal(format!(
            "embeddings API returned {} vectors for {} inputs",
            vectors.len(),
            texts.len()
        )));
    }
    Ok(vectors)
}

/// A caller-supplied vector must match the configured width, or Qdrant rejects
/// the whole request with an opaque 400. Checking here names the real problem
/// and costs nothing.
fn check_width(cfg: &Config, vector: &[f32]) -> Result<(), RagError> {
    let expected = cfg.embedding.dimensions as usize;
    if vector.len() == expected {
        return Ok(());
    }
    Err(RagError::SchemaInvalid(format!(
        "vector has {} dimensions, collection expects {expected}",
        vector.len()
    )))
}

fn ensure<H: HostCalls>(
    host: &H,
    cfg: &Config,
    collection: &str,
    dimensions: u32,
    distance: &str,
    qdrant_key: &str,
) -> Result<(), RagError> {
    let req = ensure_collection_request(
        &cfg.qdrant_url,
        collection,
        dimensions,
        distance,
        qdrant_key,
    );
    let resp = send(host, &req)?;
    parse_ensure_ack(resp.status, &resp.body)
}

/// # Errors
/// Propagates secret, transport, and Qdrant/embedding errors; returns
/// [`RagError::SchemaInvalid`] for a caller vector of the wrong width.
pub fn search<H: HostCalls>(
    host: &H,
    cfg: &Config,
    input: &SearchInput,
) -> Result<Value, RagError> {
    let vector = match (&input.vector, &input.query) {
        (Some(v), _) => {
            check_width(cfg, v)?;
            v.clone()
        }
        (None, Some(q)) => embed_all(host, cfg, std::slice::from_ref(q))?
            .into_iter()
            .next()
            .ok_or_else(|| RagError::Internal("embeddings API returned no vector".into()))?,
        // `parse_search` already rejects this; keep the arm total rather than
        // panicking on an unreachable branch.
        (None, None) => {
            return Err(RagError::InvalidInput(
                "rag_search: pass either a query or a vector".into(),
            ));
        }
    };

    let key = secret(host, QDRANT_KEY_REF)?;
    let req = query_request(
        &cfg.qdrant_url,
        collection_of(cfg, input.collection.as_ref()),
        &vector,
        input.top_k,
        input.filter.as_ref(),
        &key,
    );
    let resp = send(host, &req)?;
    let hits = parse_hits(resp.status, &resp.body)?;

    let hits: Vec<Value> = hits
        .into_iter()
        .map(|h| serde_json::json!({ "id": h.id, "score": h.score, "payload": h.payload }))
        .collect();
    Ok(serde_json::json!({ "hits": hits }))
}

/// # Errors
/// Propagates secret, transport, embedding and Qdrant errors.
pub fn upsert<H: HostCalls>(
    host: &H,
    cfg: &Config,
    input: &UpsertInput,
) -> Result<Value, RagError> {
    let vector = match (&input.vector, &input.text) {
        (Some(v), _) => {
            check_width(cfg, v)?;
            v.clone()
        }
        (None, Some(t)) => embed_all(host, cfg, std::slice::from_ref(t))?
            .into_iter()
            .next()
            .ok_or_else(|| RagError::Internal("embeddings API returned no vector".into()))?,
        (None, None) => {
            return Err(RagError::InvalidInput(
                "rag_upsert: pass either text or a vector".into(),
            ));
        }
    };

    let key = secret(host, QDRANT_KEY_REF)?;
    let collection = collection_of(cfg, input.collection.as_ref());
    ensure(
        host,
        cfg,
        collection,
        cfg.embedding.dimensions,
        "Cosine",
        &key,
    )?;

    let mut payload = input.payload.clone();
    if let (Some(map), Some(text)) = (payload.as_object_mut(), input.text.as_ref()) {
        map.insert("text".to_string(), Value::String(text.clone()));
    }

    let point = Point {
        id: input.id.clone(),
        vector,
        payload,
    };
    let req = upsert_request(&cfg.qdrant_url, collection, &[point], &key);
    let resp = send(host, &req)?;
    parse_ack(resp.status, &resp.body)?;
    Ok(serde_json::json!({ "ok": true, "id": input.id }))
}

/// Chunk, embed, then **delete the document's existing chunks before writing
/// the new ones**. Without the delete, re-ingesting a shortened document leaves
/// its tail chunks in the collection forever, still matching searches.
///
/// # Errors
/// Propagates secret, transport, embedding and Qdrant errors; returns
/// [`RagError::InvalidInput`] if chunking yields nothing.
pub fn ingest<H: HostCalls>(
    host: &H,
    cfg: &Config,
    input: &IngestInput,
) -> Result<Value, RagError> {
    let chunks = chunk_text(&input.text, cfg.chunk.max_chars, cfg.chunk.overlap_chars);
    if chunks.is_empty() {
        return Err(RagError::InvalidInput(
            "rag_ingest: text produced no chunks".into(),
        ));
    }

    let vectors = embed_all(host, cfg, &chunks)?;
    let key = secret(host, QDRANT_KEY_REF)?;
    let collection = collection_of(cfg, input.collection.as_ref());

    ensure(
        host,
        cfg,
        collection,
        cfg.embedding.dimensions,
        "Cosine",
        &key,
    )?;

    // Delete first. See the doc comment above — the order is the point.
    let del = delete_request(
        &cfg.qdrant_url,
        collection,
        &DeleteSelector::DocId(input.doc_id.clone()),
        &key,
    );
    let del_resp = send(host, &del)?;
    parse_ack(del_resp.status, &del_resp.body)?;

    let points: Vec<Point> = chunks
        .iter()
        .zip(vectors)
        .enumerate()
        .map(|(index, (text, vector))| {
            let mut payload = input.metadata.clone();
            if let Some(map) = payload.as_object_mut() {
                map.insert("doc_id".to_string(), Value::String(input.doc_id.clone()));
                map.insert("chunk_index".to_string(), Value::from(index));
                map.insert("text".to_string(), Value::String(text.clone()));
            }
            Point {
                id: chunk_point_id(&input.doc_id, index),
                vector,
                payload,
            }
        })
        .collect();

    let req = upsert_request(&cfg.qdrant_url, collection, &points, &key);
    let resp = send(host, &req)?;
    parse_ack(resp.status, &resp.body)?;

    Ok(serde_json::json!({
        "ok": true,
        "doc_id": input.doc_id,
        "chunks": points.len(),
    }))
}

/// # Errors
/// Propagates secret, transport and Qdrant errors.
pub fn delete<H: HostCalls>(
    host: &H,
    cfg: &Config,
    input: &DeleteInput,
) -> Result<Value, RagError> {
    let selector = match (&input.ids, &input.doc_id) {
        (Some(ids), _) if !ids.is_empty() => DeleteSelector::Ids(ids.clone()),
        (_, Some(doc_id)) => DeleteSelector::DocId(doc_id.clone()),
        _ => {
            return Err(RagError::InvalidInput(
                "rag_delete: pass either ids or doc_id".into(),
            ));
        }
    };
    let key = secret(host, QDRANT_KEY_REF)?;
    let req = delete_request(
        &cfg.qdrant_url,
        collection_of(cfg, input.collection.as_ref()),
        &selector,
        &key,
    );
    let resp = send(host, &req)?;
    parse_ack(resp.status, &resp.body)?;
    Ok(serde_json::json!({ "ok": true }))
}

/// # Errors
/// Propagates secret, transport and Qdrant errors.
pub fn ensure_collection<H: HostCalls>(
    host: &H,
    cfg: &Config,
    input: &EnsureInput,
) -> Result<Value, RagError> {
    let key = secret(host, QDRANT_KEY_REF)?;
    let collection = collection_of(cfg, input.collection.as_ref());
    let dimensions = input.dimensions.unwrap_or(cfg.embedding.dimensions);
    ensure(host, cfg, collection, dimensions, &input.distance, &key)?;
    Ok(serde_json::json!({
        "ok": true,
        "collection": collection,
        "dimensions": dimensions,
        "distance": input.distance,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ChunkConfig, Config, EmbeddingConfig};
    use greentic_extension_sdk_testing::mock_host::{
        CannedResponse, MockHttpClient, MockSecretsBackend,
    };

    /// Adapts the SDK's mocks to the extension's `HostCalls` trait.
    struct TestHost {
        http: MockHttpClient,
        secrets: MockSecretsBackend,
    }

    impl HostCalls for TestHost {
        fn fetch(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
            let headers: Vec<(&str, &str)> = req
                .headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let resp = self
                .http
                .fetch(&req.method, &req.url, &headers, req.body.clone());
            Ok(HttpResponse {
                status: resp.status,
                body: resp.body,
            })
        }

        fn secret(&self, uri: &str) -> Result<String, String> {
            self.secrets.get(uri).map_err(|e| e.to_string())
        }
    }

    const BASE: &str = "https://c.qdrant.io:6333";
    const EMBED_URL: &str = "https://api.openai.com/v1/embeddings";

    fn cfg() -> Config {
        Config {
            qdrant_url: BASE.to_string(),
            collection: "kb".to_string(),
            embedding: EmbeddingConfig {
                base_url: "https://api.openai.com/v1".to_string(),
                model: "text-embedding-3-small".to_string(),
                dimensions: 3,
            },
            chunk: ChunkConfig {
                max_chars: 10,
                overlap_chars: 2,
            },
        }
    }

    fn ok(json: &str) -> CannedResponse {
        CannedResponse {
            status: 200,
            body: json.as_bytes().to_vec(),
        }
    }

    /// `n` embedding vectors of the configured width. The mock keys canned
    /// responses by `(method, url)` alone, so a test that embeds three chunks
    /// needs a three-vector response — `embed_all` rejects a count mismatch.
    fn embed_ok(n: usize) -> CannedResponse {
        let data: Vec<serde_json::Value> = (0..n)
            .map(|i| serde_json::json!({"index": i, "embedding": [0.1, 0.2, 0.3]}))
            .collect();
        CannedResponse {
            status: 200,
            body: serde_json::json!({ "data": data }).to_string().into_bytes(),
        }
    }

    /// A host with both secrets present and every Qdrant/embedding call happy.
    /// `embeddings` is how many vectors the embeddings endpoint will return.
    fn happy_host(embeddings: usize) -> TestHost {
        let http = MockHttpClient::new();
        let secrets = MockSecretsBackend::new();
        secrets.set(QDRANT_KEY_REF, "qk");
        secrets.set(EMBEDDING_KEY_REF, "ek");

        http.expect("POST", EMBED_URL, embed_ok(embeddings));
        http.expect("PUT", &format!("{BASE}/collections/kb"), ok(r#"{"result":true}"#));
        http.expect(
            "PUT",
            &format!("{BASE}/collections/kb/points?wait=true"),
            ok(r#"{"status":"ok"}"#),
        );
        http.expect(
            "POST",
            &format!("{BASE}/collections/kb/points/delete?wait=true"),
            ok(r#"{"status":"ok"}"#),
        );
        http.expect(
            "POST",
            &format!("{BASE}/collections/kb/points/query"),
            ok(r#"{"result":{"points":[{"id":"p1","score":0.9,"payload":{"text":"hi"}}]}}"#),
        );
        TestHost { http, secrets }
    }

    #[test]
    fn search_by_text_embeds_first_then_queries() {
        let host = happy_host(1);
        let input = crate::input::parse_search(r#"{"query":"halo","top_k":2}"#).unwrap();
        let out = search(&host, &cfg(), &input).unwrap();

        let calls = host.http.calls();
        assert_eq!(calls.len(), 2, "expected embed then query");
        assert_eq!(calls[0].url, EMBED_URL);
        assert_eq!(calls[1].url, format!("{BASE}/collections/kb/points/query"));
        assert_eq!(out["hits"][0]["id"], "p1");
    }

    #[test]
    fn search_by_vector_skips_the_embedding_call_entirely() {
        let host = happy_host(1);
        let input = crate::input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap();
        search(&host, &cfg(), &input).unwrap();

        let calls = host.http.calls();
        assert_eq!(calls.len(), 1);
        assert!(!calls[0].url.contains("embeddings"));
    }

    #[test]
    fn a_raw_search_vector_of_the_wrong_width_is_rejected_before_any_call() {
        let host = happy_host(1);
        let input = crate::input::parse_search(r#"{"vector":[0.1,0.2]}"#).unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::SchemaInvalid(_)), "got {err:?}");
        assert!(host.http.calls().is_empty(), "must not reach the network");
    }

    #[test]
    fn ingest_deletes_the_document_before_upserting_its_chunks() {
        // 25 chars with a 10/2 window → 3 chunks, so three vectors come back.
        let host = happy_host(3);
        let input = crate::input::parse_ingest(
            r#"{"doc_id":"d1","text":"abcdefghijklmnopqrstuvwxy"}"#,
        )
        .unwrap();
        let out = ingest(&host, &cfg(), &input).unwrap();

        let urls: Vec<String> = host.http.calls().into_iter().map(|c| c.url).collect();
        let delete_at = urls
            .iter()
            .position(|u| u.contains("/points/delete"))
            .expect("no delete call");
        let upsert_at = urls
            .iter()
            .position(|u| u.contains("/points?wait=true"))
            .expect("no upsert call");
        assert!(
            delete_at < upsert_at,
            "delete must precede upsert, got {urls:?}"
        );
        assert_eq!(out["chunks"], 3);
    }

    #[test]
    fn ingest_writes_deterministic_uuid_ids_carrying_doc_id_and_index() {
        let host = happy_host(1);
        let input =
            crate::input::parse_ingest(r#"{"doc_id":"d1","text":"short"}"#).unwrap();
        ingest(&host, &cfg(), &input).unwrap();

        let upsert = host
            .http
            .calls()
            .into_iter()
            .find(|c| c.url.contains("/points?wait=true"))
            .expect("no upsert call");
        let body: serde_json::Value =
            serde_json::from_slice(upsert.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["points"][0]["id"], crate::qdrant::chunk_point_id("d1", 0));
        assert_eq!(body["points"][0]["payload"]["doc_id"], "d1");
        assert_eq!(body["points"][0]["payload"]["chunk_index"], 0);
        assert_eq!(body["points"][0]["payload"]["text"], "short");
    }

    #[test]
    fn ingest_forwards_caller_metadata_into_every_chunk_payload() {
        let host = happy_host(1);
        let input = crate::input::parse_ingest(
            r#"{"doc_id":"d1","text":"short","metadata":{"lang":"id"}}"#,
        )
        .unwrap();
        ingest(&host, &cfg(), &input).unwrap();

        let upsert = host
            .http
            .calls()
            .into_iter()
            .find(|c| c.url.contains("/points?wait=true"))
            .expect("no upsert call");
        let body: serde_json::Value =
            serde_json::from_slice(upsert.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["points"][0]["payload"]["lang"], "id");
    }

    #[test]
    fn a_missing_secret_surfaces_as_permission_denied() {
        let http = MockHttpClient::new();
        let secrets = MockSecretsBackend::new(); // nothing set
        let host = TestHost { http, secrets };
        let input = crate::input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::PermissionDenied(_)), "got {err:?}");
    }

    #[test]
    fn nothing_is_requested_outside_the_declared_network_allowlist() {
        let host = happy_host(1);
        // Exactly what describe.json grants.
        host.http
            .restrict_to_hosts(&["c.qdrant.io".to_string(), "api.openai.com".to_string()]);
        let input = crate::input::parse_ingest(r#"{"doc_id":"d1","text":"short"}"#).unwrap();
        ingest(&host, &cfg(), &input).unwrap();
        for call in host.http.calls() {
            assert!(
                call.url.contains("c.qdrant.io") || call.url.contains("api.openai.com"),
                "unexpected host: {}",
                call.url
            );
        }
    }

    #[test]
    fn delete_and_ensure_return_an_acknowledgement() {
        let host = happy_host(1);
        let del = crate::input::parse_delete(r#"{"doc_id":"d1"}"#).unwrap();
        assert_eq!(delete(&host, &cfg(), &del).unwrap()["ok"], true);

        let ens = crate::input::parse_ensure(r#"{}"#).unwrap();
        assert_eq!(ensure_collection(&host, &cfg(), &ens).unwrap()["ok"], true);
    }

    #[test]
    fn a_collection_override_is_honoured_over_the_configured_default() {
        let host = happy_host(1);
        host.http.expect(
            "POST",
            &format!("{BASE}/collections/other/points/query"),
            ok(r#"{"result":{"points":[]}}"#),
        );
        let input =
            crate::input::parse_search(r#"{"vector":[0.1,0.2,0.3],"collection":"other"}"#)
                .unwrap();
        search(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/other/"));
    }
}
