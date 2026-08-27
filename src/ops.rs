//! Orchestration: the only place in the extension that issues more than one
//! host call per tool. Generic over `HostCalls` so every sequence is testable.

use serde_json::Value;

use crate::chunk::chunk_text;
use crate::config::Config;
use crate::embed::{embed_request, parse_embed_response};
use crate::error::RagError;
use crate::host::{HostCalls, HttpRequest, HttpResponse};
use crate::input::{
    DeleteInput, EnsureInput, IngestInput, ListInput, SearchInput, TenantOverlay, UpsertInput,
};
use crate::qdrant::{
    DeleteSelector, Point, ScrollPoint, chunk_point_id, delete_request, ensure_collection_request,
    parse_ack, parse_ensure_ack, parse_hits, parse_scroll, query_request, scroll_request,
    upsert_request,
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
    host.secret(uri)
        .map_err(|e| RagError::PermissionDenied(format!("host could not resolve {uri}: {e}")))
}

/// Resolve which Qdrant collection this call reads and writes.
///
/// Precedence, highest first:
///
/// 1. `_tenant_overlay.collection` — the host's per-tenant config. Host-set
///    and unforgeable (see [`TenantOverlay`]), so it is the isolation
///    boundary and nothing a caller says may move it.
/// 2. the caller's `collection` argument — only when no overlay pins one.
/// 3. `cfg.collection`, which [`crate::config::resolve`] has already filled in
///    from the overlay or from a `lifecycle::init` baseline. By the time it
///    gets here, step 1 and step 3 may well be the same string — step 1 still
///    reads the overlay directly, because whether the overlay *pinned* a
///    collection is what decides the refusal below, and a merged value can no
///    longer be traced back to its source.
///
/// **Any caller `collection` is refused while an overlay pins one, even one
/// that matches.** Ignoring a disagreement silently would let a flow author
/// believe they were reading a collection they were not — the failure this
/// whole ordering exists to prevent. Refusing only a *disagreement* would
/// leave the matching case as a quiet invitation to hard-code a tenant's
/// collection name into a flow, which reviews cleanly, works until the tenant
/// is reconfigured or the flow is copied to another tenant, and then breaks
/// somewhere far away. One total rule — "when the host pins the collection,
/// the argument is not yours to set" — is easier to document, to test, and to
/// act on than a rule with an exception in it.
///
/// Step 2 is what keeps single-tenant and local development working: with no
/// overlay at all there is nothing to undermine, and the argument behaves
/// exactly as it always has.
///
/// # Errors
/// [`RagError::InvalidInput`] if the caller passed `collection` while the
/// overlay pins one, if the overlay's collection is blank, if no collection is
/// configured at all, or if the overlay carries a `qdrant_url` that is not the
/// one `cfg` was resolved with.
/// [`RagError::PermissionDenied`] if the operator requires a tenant overlay
/// and none arrived.
fn collection_of<'a>(
    cfg: &'a Config,
    overlay: Option<&'a TenantOverlay>,
    override_: Option<&'a String>,
) -> Result<&'a str, RagError> {
    let pinned = overlay.and_then(|o| o.collection.as_deref());

    // Every request builder below reads `cfg.qdrant_url`, so this tenant's
    // collection must only ever be addressed on the cluster this call's own
    // overlay named. `config::resolve` guarantees that: when the overlay
    // carries a url, the resolved `cfg.qdrant_url` *is* it (or resolution
    // already refused, because `lifecycle::init` had pinned a different one).
    //
    // Keeping the comparison here anyway costs one string compare and closes
    // the gap that would open the moment someone hands this function a `cfg`
    // resolved from a *different* call's overlay: a mismatch would mean
    // reading one tenant's collection name off another tenant's cluster,
    // which is exactly the cross-tenant read this ordering exists to prevent.
    if let Some(url) = overlay.and_then(|o| o.qdrant_url.as_deref())
        && url.trim().trim_end_matches('/') != cfg.qdrant_url
    {
        return Err(RagError::InvalidInput(format!(
            "tenant overlay sets qdrant_url {url:?}, which is not the {:?} this call was \
             configured with. Refusing rather than reading this tenant's collection off \
             another cluster.",
            cfg.qdrant_url
        )));
    }

    match pinned {
        Some(pinned) => {
            if pinned.trim().is_empty() {
                return Err(RagError::InvalidInput(
                    "tenant overlay pins an empty collection name; fix this tenant's extension \
                     configuration"
                        .into(),
                ));
            }
            if let Some(requested) = override_ {
                return Err(RagError::InvalidInput(format!(
                    "collection {requested:?} was passed, but this tenant's collection is set \
                     by the host ({pinned:?}) and cannot be overridden per call. Remove the \
                     `collection` argument."
                )));
            }
            Ok(pinned)
        }
        // No overlay collection. Single-tenant installs and local development
        // land here and keep their per-call override.
        None => {
            if cfg.require_tenant_overlay {
                return Err(RagError::PermissionDenied(
                    "this instance sets require_tenant_overlay, but the host sent no tenant \
                     collection for this call. Refusing rather than falling back to the shared \
                     configured collection."
                        .into(),
                ));
            }
            let chosen = override_.map_or(cfg.collection.as_str(), String::as_str);
            // Nothing named a collection: no overlay, no argument, and no
            // baseline. Addressing `/collections//points/...` would be a
            // confusing 404 from Qdrant; say what to configure instead.
            if chosen.trim().is_empty() {
                return Err(crate::config::not_configured("collection"));
            }
            Ok(chosen)
        }
    }
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
    // Resolved first, ahead of any host call. Embedding before deciding which
    // collection the call is even allowed to touch would bill the operator for
    // an embeddings request that is about to be refused — and, worse, put a
    // side effect before an authorisation check.
    let collection = collection_of(
        cfg,
        input.tenant_overlay.as_ref(),
        input.collection.as_ref(),
    )?;

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
        collection,
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
    // Same reasoning as `ingest`: a non-object payload silently swallows the
    // `text` insert instead of failing. Reject it before any host call.
    if !input.payload.is_object() {
        return Err(RagError::InvalidInput(
            "rag_upsert: payload must be a JSON object".into(),
        ));
    }

    // Resolved first, ahead of any host call. Embedding before deciding which
    // collection the call is even allowed to touch would bill the operator for
    // an embeddings request that is about to be refused — and, worse, put a
    // side effect before an authorisation check.
    let collection = collection_of(
        cfg,
        input.tenant_overlay.as_ref(),
        input.collection.as_ref(),
    )?;

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
    // A non-object metadata value would make `as_object_mut()` below return
    // `None`, silently dropping the `doc_id` insert. The point would still be
    // written — but with no `doc_id`, so `delete_request`'s filter could never
    // find it again. That is the orphan chunk this whole function is ordered to
    // prevent, arriving through a different door. Reject it before any host call.
    if !input.metadata.is_object() {
        return Err(RagError::InvalidInput(
            "rag_ingest: metadata must be a JSON object".into(),
        ));
    }

    // Resolved before chunking and embedding. A refusal here must not cost the
    // operator an embeddings call for every chunk of the document, and an
    // authorisation check belongs ahead of the side effects, not between them.
    let collection = collection_of(
        cfg,
        input.tenant_overlay.as_ref(),
        input.collection.as_ref(),
    )?;

    let chunks = chunk_text(&input.text, cfg.chunk.max_chars, cfg.chunk.overlap_chars);
    if chunks.is_empty() {
        return Err(RagError::InvalidInput(
            "rag_ingest: text produced no chunks".into(),
        ));
    }

    let vectors = embed_all(host, cfg, &chunks)?;
    let key = secret(host, QDRANT_KEY_REF)?;

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
        collection_of(
            cfg,
            input.tenant_overlay.as_ref(),
            input.collection.as_ref(),
        )?,
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
    let collection = collection_of(
        cfg,
        input.tenant_overlay.as_ref(),
        input.collection.as_ref(),
    )?;
    let dimensions = input.dimensions.unwrap_or(cfg.embedding.dimensions);
    ensure(host, cfg, collection, dimensions, &input.distance, &key)?;
    Ok(serde_json::json!({
        "ok": true,
        "collection": collection,
        "dimensions": dimensions,
        "distance": input.distance,
    }))
}

/// A payload's `doc_id`, `chunk_index` and `text` are chunk artifacts written
/// by `ingest` (see the payload assembly there), not part of the caller's
/// original metadata. Stripping them back out recovers exactly the object the
/// caller passed as `metadata` — for a `rag_ingest`-created point, whose
/// payload assembly clones `input.metadata` verbatim. `rag_upsert` accepts an
/// arbitrary payload with no such guarantee, so for a point written that way
/// this only strips the three reserved keys back out of whatever was there. A
/// non-object payload (should not happen — `upsert` and `ingest` both reject
/// one before writing) is returned unchanged rather than discarded.
fn strip_chunk_fields(payload: &Value) -> Value {
    let Some(map) = payload.as_object() else {
        return payload.clone();
    };
    let mut metadata = map.clone();
    metadata.remove("doc_id");
    metadata.remove("chunk_index");
    metadata.remove("text");
    Value::Object(metadata)
}

/// A point's rank within its `doc_id` group, ascending — the lowest rank
/// wins. `(false, ..)` (has a `chunk_index`) sorts before `(true, ..)` (does
/// not), so any point with a `chunk_index` outranks any point without one
/// regardless of the index's value. Within each of those two bands, ties are
/// broken by the numeric `chunk_index` and then, for points that share both a
/// band and an index (or lack one entirely), by the point id string —
/// giving a total order that is independent of scroll return order.
type ChunkRank = (bool, u64, String);

fn chunk_rank(point: &ScrollPoint) -> ChunkRank {
    let chunk_index = point.payload.get("chunk_index").and_then(Value::as_u64);
    (
        chunk_index.is_none(),
        chunk_index.unwrap_or(0),
        point.id.clone(),
    )
}

/// Per-`doc_id` accumulator for [`list`]: every point in the group is
/// counted, but only the metadata of the point with the lowest [`ChunkRank`]
/// survives.
struct DocGroup {
    chunk_count: u32,
    best_rank: ChunkRank,
    metadata: Value,
}

/// Enumerate stored documents, one Qdrant scroll page at a time, grouped by
/// `doc_id` rather than by chunk.
///
/// A chunk count is only for the chunks that landed in *this* page — a
/// document whose chunks straddle a page boundary shows a partial count on
/// each page it appears in. That is the honest answer to "how many chunks on
/// this page", not "how many chunks does this document have in total";
/// getting the total would mean looping every page inside the tool, which is
/// the truncation-by-silence failure mode this tool exists to avoid.
///
/// A point with no `doc_id` in its payload — `upsert` does not require one —
/// is grouped under its own point id, so it still shows up as a one-chunk
/// entry instead of disappearing from the listing.
///
/// Only one point's metadata survives per `doc_id` — payloads are never
/// merged. The surviving point is the one with the lowest `chunk_index`;
/// points with no `chunk_index` rank after every point that has one, and
/// among themselves are ordered by their point id string, ascending. This
/// makes the winner deterministic regardless of the order Qdrant's scroll
/// happens to return points in. For a `rag_ingest`-created document every
/// chunk carries identical metadata, so the rule is invisible. For a document
/// assembled from several `rag_upsert` calls sharing a `doc_id` but carrying
/// different payloads, only the winning point's metadata is returned — the
/// rest are silently discarded, by design, not merged.
///
/// # Errors
/// Propagates secret, transport and Qdrant errors.
pub fn list<H: HostCalls>(host: &H, cfg: &Config, input: &ListInput) -> Result<Value, RagError> {
    let key = secret(host, QDRANT_KEY_REF)?;
    let req = scroll_request(
        &cfg.qdrant_url,
        collection_of(
            cfg,
            input.tenant_overlay.as_ref(),
            input.collection.as_ref(),
        )?,
        input.limit,
        input.offset.as_ref(),
        input.filter.as_ref(),
        &key,
    );
    let resp = send(host, &req)?;
    let page = parse_scroll(resp.status, &resp.body)?;

    let mut order: Vec<String> = Vec::new();
    let mut by_doc: std::collections::HashMap<String, DocGroup> = std::collections::HashMap::new();
    for point in page.points {
        let doc_id = point
            .payload
            .get("doc_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| point.id.clone());
        let rank = chunk_rank(&point);
        match by_doc.get_mut(&doc_id) {
            Some(group) => {
                group.chunk_count += 1;
                if rank < group.best_rank {
                    group.best_rank = rank;
                    group.metadata = strip_chunk_fields(&point.payload);
                }
            }
            None => {
                order.push(doc_id.clone());
                by_doc.insert(
                    doc_id,
                    DocGroup {
                        chunk_count: 1,
                        best_rank: rank,
                        metadata: strip_chunk_fields(&point.payload),
                    },
                );
            }
        }
    }

    let documents: Vec<Value> = order
        .into_iter()
        .filter_map(|doc_id| {
            by_doc.remove(&doc_id).map(|group| {
                serde_json::json!({
                    "doc_id": doc_id,
                    "chunk_count": group.chunk_count,
                    "metadata": group.metadata,
                })
            })
        })
        .collect();

    let mut out = serde_json::Map::new();
    out.insert("documents".to_string(), Value::Array(documents));
    if let Some(next_page_offset) = page.next_page_offset {
        out.insert("next_page_offset".to_string(), next_page_offset);
    }
    Ok(Value::Object(out))
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
            require_tenant_overlay: false,
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
        http.expect(
            "PUT",
            &format!("{BASE}/collections/kb"),
            ok(r#"{"result":true}"#),
        );
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
        let input =
            crate::input::parse_ingest(r#"{"doc_id":"d1","text":"abcdefghijklmnopqrstuvwxy"}"#)
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
        let input = crate::input::parse_ingest(r#"{"doc_id":"d1","text":"short"}"#).unwrap();
        ingest(&host, &cfg(), &input).unwrap();

        let upsert = host
            .http
            .calls()
            .into_iter()
            .find(|c| c.url.contains("/points?wait=true"))
            .expect("no upsert call");
        let body: serde_json::Value =
            serde_json::from_slice(upsert.body.as_deref().unwrap()).unwrap();
        assert_eq!(
            body["points"][0]["id"],
            crate::qdrant::chunk_point_id("d1", 0)
        );
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

    /// A non-object metadata value must not reach Qdrant. If it did, the
    /// doc_id insert would be skipped and the chunk would be undeletable.
    #[test]
    fn ingest_rejects_non_object_metadata_before_any_call() {
        let host = happy_host(1);
        let input = crate::input::parse_ingest(r#"{"doc_id":"d1","text":"short","metadata":null}"#)
            .unwrap();
        let err = ingest(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(host.http.calls().is_empty(), "must not reach the network");
    }

    #[test]
    fn upsert_rejects_non_object_payload_before_any_call() {
        let host = happy_host(1);
        let input =
            crate::input::parse_upsert(r#"{"id":"1","text":"hi","payload":[1,2]}"#).unwrap();
        let err = upsert(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(host.http.calls().is_empty(), "must not reach the network");
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
        // These are exactly what describe.json grants: `*.qdrant.io` covers
        // any Qdrant Cloud tenant, and `api.openai.com` is the embeddings host.
        host.http
            .restrict_to_hosts(&["*.qdrant.io".to_string(), "api.openai.com".to_string()]);
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
            crate::input::parse_search(r#"{"vector":[0.1,0.2,0.3],"collection":"other"}"#).unwrap();
        search(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/other/"));
    }

    /// A host with only the Qdrant secret set, whose scroll endpoint returns
    /// `scroll_response` for `collection`. `rag_list` never embeds, so unlike
    /// `happy_host` there is no embeddings-endpoint expectation to satisfy.
    fn list_host(scroll_response: CannedResponse, collection: &str) -> TestHost {
        let http = MockHttpClient::new();
        let secrets = MockSecretsBackend::new();
        secrets.set(QDRANT_KEY_REF, "qk");
        http.expect(
            "POST",
            &format!("{BASE}/collections/{collection}/points/scroll"),
            scroll_response,
        );
        TestHost { http, secrets }
    }

    #[test]
    fn list_groups_several_chunks_of_two_documents_into_two_entries() {
        let body = serde_json::json!({
            "result": {
                "points": [
                    {"id": "p1", "payload": {"doc_id": "d1", "chunk_index": 0, "text": "a"}},
                    {"id": "p2", "payload": {"doc_id": "d1", "chunk_index": 1, "text": "b"}},
                    {"id": "p3", "payload": {"doc_id": "d2", "chunk_index": 0, "text": "c", "lang": "id"}},
                ],
                "next_page_offset": null,
            }
        })
        .to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();

        let docs = out["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 2, "two doc_ids must collapse to two entries");

        let d1 = docs.iter().find(|d| d["doc_id"] == "d1").unwrap();
        assert_eq!(d1["chunk_count"], 2);

        let d2 = docs.iter().find(|d| d["doc_id"] == "d2").unwrap();
        assert_eq!(d2["chunk_count"], 1);
        assert_eq!(d2["metadata"]["lang"], "id");
        // Chunk artifacts must not leak into the reported metadata.
        assert!(d2["metadata"].get("text").is_none());
        assert!(d2["metadata"].get("chunk_index").is_none());
        assert!(d2["metadata"].get("doc_id").is_none());
    }

    #[test]
    fn list_groups_a_point_with_no_doc_id_under_its_own_point_id() {
        // rag_upsert does not require a doc_id, so a point written that way
        // must still show up as a one-chunk entry, not vanish from the list.
        let body = serde_json::json!({
            "result": {
                "points": [{"id": "p1", "payload": {"lang": "id"}}],
                "next_page_offset": null,
            }
        })
        .to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();

        let docs = out["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["doc_id"], "p1");
        assert_eq!(docs[0]["chunk_count"], 1);
    }

    /// `rag_upsert` lets several calls share a `doc_id` with different
    /// payloads (see `UPSERT_META.usage_hint`), so the point Qdrant happens
    /// to scroll first is not necessarily the one whose metadata should
    /// survive. The winner must be the lowest `chunk_index`, regardless of
    /// the order the points arrive in — here deliberately out of order
    /// (index 2, then 0, then 1).
    #[test]
    fn list_picks_the_lowest_chunk_index_metadata_regardless_of_scroll_order() {
        let body = serde_json::json!({
            "result": {
                "points": [
                    {"id": "pc", "payload": {"doc_id": "d1", "chunk_index": 2, "marker": "idx2"}},
                    {"id": "pa", "payload": {"doc_id": "d1", "chunk_index": 0, "marker": "idx0"}},
                    {"id": "pb", "payload": {"doc_id": "d1", "chunk_index": 1, "marker": "idx1"}},
                ],
                "next_page_offset": null,
            }
        })
        .to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();

        let docs = out["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(
            docs[0]["chunk_count"], 3,
            "every point in the group must still be counted"
        );
        assert_eq!(
            docs[0]["metadata"]["marker"], "idx0",
            "the surviving metadata must be chunk_index 0's, not the first point Qdrant returned"
        );
    }

    /// Points written by `rag_upsert` need not carry a `chunk_index` at all.
    /// Among such points sharing a `doc_id`, the tie-break is the point id
    /// string, ascending — again independent of scroll order.
    #[test]
    fn list_breaks_ties_among_indexless_points_by_point_id_ascending() {
        let body = serde_json::json!({
            "result": {
                "points": [
                    {"id": "zzz", "payload": {"doc_id": "d1", "marker": "from-zzz"}},
                    {"id": "aaa", "payload": {"doc_id": "d1", "marker": "from-aaa"}},
                ],
                "next_page_offset": null,
            }
        })
        .to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();

        let docs = out["documents"].as_array().unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0]["chunk_count"], 2);
        assert_eq!(
            docs[0]["metadata"]["marker"], "from-aaa",
            "\"aaa\" sorts before \"zzz\", so its metadata must win even though \
             \"zzz\" was scrolled first"
        );
    }

    #[test]
    fn list_surfaces_the_pagination_offset_when_qdrant_returns_one() {
        let body = serde_json::json!({
            "result": {
                "points": [],
                "next_page_offset": "3f2504e0-4f89-41d3-9a0c-0305e82c3301",
            }
        })
        .to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();
        assert_eq!(
            out["next_page_offset"],
            "3f2504e0-4f89-41d3-9a0c-0305e82c3301"
        );
    }

    #[test]
    fn list_omits_the_pagination_offset_key_entirely_when_qdrant_has_no_more_pages() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();
        assert!(
            out.as_object().unwrap().get("next_page_offset").is_none(),
            "next_page_offset must be absent, not null, once pages are exhausted: {out}"
        );
    }

    #[test]
    fn list_on_an_empty_collection_returns_an_empty_list_not_an_error() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "kb");
        let input = crate::input::parse_list(r#"{}"#).unwrap();
        let out = list(&host, &cfg(), &input).unwrap();
        assert_eq!(out["documents"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn list_collection_override_reaches_the_request_url() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "other");
        let input = crate::input::parse_list(r#"{"collection":"other"}"#).unwrap();
        list(&host, &cfg(), &input).unwrap();
        assert!(
            host.http.calls()[0]
                .url
                .contains("/collections/other/points/scroll")
        );
    }

    // ---- tenant overlay -------------------------------------------------
    //
    // `_tenant_overlay` is stamped by the host on every call and stripped from
    // the caller's own args first, so from in here it is trusted input. These
    // tests pin the precedence it establishes over the `collection` argument.

    fn cfg_requiring_overlay() -> Config {
        Config {
            require_tenant_overlay: true,
            ..cfg()
        }
    }

    #[test]
    fn the_tenant_overlay_collection_reaches_the_request_url() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "tenant-a");
        let input =
            crate::input::parse_list(r#"{"_tenant_overlay":{"collection":"tenant-a"}}"#).unwrap();
        list(&host, &cfg(), &input).unwrap();
        assert!(
            host.http.calls()[0]
                .url
                .contains("/collections/tenant-a/points/scroll")
        );
    }

    #[test]
    fn the_overlay_outranks_the_configured_default_without_any_caller_argument() {
        let host = happy_host(1);
        host.http.expect(
            "POST",
            &format!("{BASE}/collections/tenant-a/points/query"),
            ok(r#"{"result":{"points":[]}}"#),
        );
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],"_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        search(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/tenant-a/"));
    }

    /// The whole point of the ordering: a caller cannot read another tenant's
    /// collection by naming it, and is told so rather than quietly served the
    /// right one.
    #[test]
    fn a_caller_collection_is_refused_while_the_overlay_pins_one() {
        let host = happy_host(1);
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],"collection":"tenant-b","_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(
            matches!(err, RagError::InvalidInput(ref m)
                if m.contains("tenant-b") && m.contains("tenant-a")),
            "unexpected error: {err:?}"
        );
        assert!(
            host.http.calls().is_empty(),
            "refusal must happen before any host call"
        );
    }

    /// Refused even when it agrees. See `collection_of`: one total rule, and
    /// a matching override is an invitation to hard-code a tenant's collection
    /// into a flow that will be copied to another tenant later.
    #[test]
    fn a_caller_collection_matching_the_overlay_is_refused_too() {
        let host = happy_host(1);
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],"collection":"tenant-a","_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(host.http.calls().is_empty());
    }

    /// Regression: the refusal used to land *after* the embeddings call on the
    /// text path, so a rejected search still billed the operator for an
    /// embedding — a side effect before an authorisation check. The
    /// vector-path tests above cannot catch this, because they never embed.
    #[test]
    fn a_refused_search_on_the_text_path_never_reaches_the_embeddings_api() {
        let host = happy_host(1);
        let input = crate::input::parse_search(
            r#"{"query":"anything","collection":"tenant-b","_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(
            host.http.calls().is_empty(),
            "no embeddings call may precede the refusal"
        );
    }

    /// Same ordering, for the tool that would embed every chunk of a document.
    #[test]
    fn a_refused_ingest_never_reaches_the_embeddings_api() {
        let host = happy_host(4);
        let input = crate::input::parse_ingest(
            r#"{"doc_id":"d1","text":"a much longer document that would chunk into several pieces","collection":"tenant-b","_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        let err = ingest(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(host.http.calls().is_empty());
    }

    /// Deleting is the destructive one; the same refusal must cover it, or a
    /// caller could aim a delete at a collection the overlay did not choose.
    #[test]
    fn delete_also_refuses_a_caller_collection_under_an_overlay() {
        let host = happy_host(0);
        let input = crate::input::parse_delete(
            r#"{"doc_id":"d1","collection":"tenant-b","_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        let err = delete(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(host.http.calls().is_empty());
    }

    #[test]
    fn ingest_also_refuses_a_caller_collection_under_an_overlay() {
        let host = happy_host(1);
        let input = crate::input::parse_ingest(
            r#"{"doc_id":"d1","text":"hello there","collection":"tenant-b","_tenant_overlay":{"collection":"tenant-a"}}"#,
        )
        .unwrap();
        let err = ingest(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(
            host.http.calls().is_empty(),
            "nothing may be embedded or written before the refusal"
        );
    }

    /// Single-tenant and local development: no overlay, so the argument keeps
    /// working exactly as it did before tenant stamping existed.
    #[test]
    fn a_caller_collection_still_works_when_no_overlay_is_present() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "other");
        let input = crate::input::parse_list(r#"{"collection":"other"}"#).unwrap();
        list(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/other/"));
    }

    /// An overlay that arrives carrying no collection is the same situation as
    /// no overlay at all — the host had nothing configured for this tenant.
    #[test]
    fn an_overlay_without_a_collection_leaves_the_caller_override_alone() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "other");
        let input = crate::input::parse_list(
            r#"{"collection":"other","_tenant_overlay":{"qdrant_url":"https://c.qdrant.io:6333"}}"#,
        )
        .unwrap();
        list(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/other/"));
    }

    #[test]
    fn a_blank_overlay_collection_is_refused_rather_than_used() {
        let host = happy_host(1);
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],"_tenant_overlay":{"collection":"   "}}"#,
        )
        .unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(matches!(err, RagError::InvalidInput(_)), "got {err:?}");
        assert!(host.http.calls().is_empty());
    }

    /// A fully-merged overlay echoes the baseline url, which must not trip the
    /// mismatch guard.
    #[test]
    fn an_overlay_repeating_the_configured_qdrant_url_is_accepted() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "tenant-a");
        let json =
            format!(r#"{{"_tenant_overlay":{{"collection":"tenant-a","qdrant_url":"{BASE}"}}}}"#);
        let input = crate::input::parse_list(&json).unwrap();
        list(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/tenant-a/"));
    }

    /// Isolating tenants by cluster is a shape this extension cannot serve:
    /// every request builder reads `cfg.qdrant_url`. Refuse loudly instead of
    /// reading a tenant's collection name off the wrong cluster.
    #[test]
    fn an_overlay_qdrant_url_that_disagrees_is_refused() {
        let host = happy_host(1);
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],"_tenant_overlay":{"collection":"tenant-a","qdrant_url":"https://elsewhere.qdrant.io:6333"}}"#,
        )
        .unwrap();
        let err = search(&host, &cfg(), &input).unwrap_err();
        assert!(
            matches!(err, RagError::InvalidInput(ref m) if m.contains("elsewhere.qdrant.io")),
            "unexpected error: {err:?}"
        );
        assert!(host.http.calls().is_empty());
    }

    /// The fail-open hole, closed by opt-in: a host that never stamps an
    /// overlay would otherwise serve every tenant the configured collection.
    #[test]
    fn require_tenant_overlay_refuses_a_call_the_host_did_not_stamp() {
        let host = happy_host(1);
        let input = crate::input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap();
        let err = search(&host, &cfg_requiring_overlay(), &input).unwrap_err();
        assert!(
            matches!(err, RagError::PermissionDenied(ref m) if m.contains("require_tenant_overlay")),
            "unexpected error: {err:?}"
        );
        assert!(host.http.calls().is_empty());
    }

    /// ...and it must not refuse a caller override either, or an operator
    /// could work around the requirement from a flow.
    #[test]
    fn require_tenant_overlay_is_not_satisfied_by_a_caller_collection() {
        let host = happy_host(1);
        let input =
            crate::input::parse_search(r#"{"vector":[0.1,0.2,0.3],"collection":"tenant-a"}"#)
                .unwrap();
        let err = search(&host, &cfg_requiring_overlay(), &input).unwrap_err();
        assert!(matches!(err, RagError::PermissionDenied(_)), "got {err:?}");
        assert!(host.http.calls().is_empty());
    }

    #[test]
    fn require_tenant_overlay_passes_once_the_host_stamps_one() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "tenant-a");
        let input =
            crate::input::parse_list(r#"{"_tenant_overlay":{"collection":"tenant-a"}}"#).unwrap();
        list(&host, &cfg_requiring_overlay(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/tenant-a/"));
    }

    /// Forward compatibility: a host that learns to send more of the config
    /// must not break a guest that has not learned to read that key yet.
    #[test]
    fn unknown_keys_inside_the_overlay_are_ignored() {
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        let host = list_host(ok(&body), "tenant-a");
        let input = crate::input::parse_list(
            r#"{"_tenant_overlay":{"collection":"tenant-a","future_key":{"nested":true}}}"#,
        )
        .unwrap();
        list(&host, &cfg(), &input).unwrap();
        assert!(host.http.calls()[0].url.contains("/collections/tenant-a/"));
    }

    // ---- configured entirely by the overlay -----------------------------
    //
    // No `lifecycle::init` anywhere: the config these run against is built the
    // way `dispatch` builds it, from the overlay alone. The tests above pass a
    // hand-written `cfg()` and so cannot show that a real deployment — where
    // the only configuration that exists is the one stamped onto the call —
    // reaches Qdrant at all.

    /// The end-to-end shape of the fix: an overlay names the cluster and the
    /// collection, and the request lands on both.
    #[test]
    fn an_overlay_alone_configures_a_call_that_reaches_the_right_cluster() {
        let cfg = crate::config::resolve(
            None,
            Some(
                &serde_json::from_str(
                    r#"{"qdrant_url":"https://tenant-a.qdrant.io:6333","collection":"tenant-a-kb"}"#,
                )
                .unwrap(),
            ),
        )
        .expect("an overlay carrying a url and a collection is a complete configuration");

        let http = MockHttpClient::new();
        let secrets = MockSecretsBackend::new();
        secrets.set(QDRANT_KEY_REF, "qk");
        let body =
            serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
        http.expect(
            "POST",
            "https://tenant-a.qdrant.io:6333/collections/tenant-a-kb/points/scroll",
            ok(&body),
        );
        let host = TestHost { http, secrets };

        let input = crate::input::parse_list(
            r#"{"_tenant_overlay":{"qdrant_url":"https://tenant-a.qdrant.io:6333","collection":"tenant-a-kb"}}"#,
        )
        .unwrap();
        list(&host, &cfg, &input).unwrap();
        assert_eq!(
            host.http.calls()[0].url,
            "https://tenant-a.qdrant.io:6333/collections/tenant-a-kb/points/scroll"
        );
    }

    /// Two tenants, two clusters, one instance — each call configured by its
    /// own overlay. Nothing may leak from the first call into the second, so
    /// the config must be per-call and never cached in a static.
    #[test]
    fn two_overlays_in_a_row_are_not_confused_with_one_another() {
        for (host_name, collection) in [
            ("tenant-a.qdrant.io", "tenant-a-kb"),
            ("tenant-b.qdrant.io", "tenant-b-kb"),
        ] {
            let url = format!("https://{host_name}:6333");
            let json = format!(
                r#"{{"_tenant_overlay":{{"qdrant_url":"{url}","collection":"{collection}"}}}}"#
            );
            let input = crate::input::parse_list(&json).unwrap();
            let cfg =
                crate::config::resolve(None, input.tenant_overlay.as_ref()).expect("must resolve");

            let http = MockHttpClient::new();
            let secrets = MockSecretsBackend::new();
            secrets.set(QDRANT_KEY_REF, "qk");
            let body =
                serde_json::json!({"result": {"points": [], "next_page_offset": null}}).to_string();
            let expected = format!("{url}/collections/{collection}/points/scroll");
            http.expect("POST", &expected, ok(&body));
            let host = TestHost { http, secrets };

            list(&host, &cfg, &input).unwrap();
            assert_eq!(host.http.calls()[0].url, expected);
        }
    }

    /// An overlay-configured call with `require_tenant_overlay` in the
    /// operator baseline still refuses when that overlay names no collection —
    /// the flag's remaining bite, now that the overlay is also the config.
    #[test]
    fn an_overlay_without_a_collection_still_trips_require_tenant_overlay() {
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],
                "_tenant_overlay":{"qdrant_url":"https://c.qdrant.io:6333",
                                   "require_tenant_overlay":true}}"#,
        )
        .unwrap();
        // Resolved exactly as `dispatch` resolves it: from this call's own
        // overlay, with no `lifecycle::init` baseline underneath.
        let cfg = crate::config::resolve(None, input.tenant_overlay.as_ref()).unwrap();

        let host = happy_host(1);
        let err = search(&host, &cfg, &input).unwrap_err();
        assert!(matches!(err, RagError::PermissionDenied(_)), "got {err:?}");
        assert!(host.http.calls().is_empty());
    }

    /// Nothing configured anywhere: no overlay collection, no argument, no
    /// baseline. The operator must be told what to fill in, not handed a
    /// request to `/collections//points/...`.
    #[test]
    fn a_call_with_no_collection_from_any_source_names_what_to_configure() {
        let input = crate::input::parse_search(
            r#"{"vector":[0.1,0.2,0.3],
                "_tenant_overlay":{"qdrant_url":"https://c.qdrant.io:6333"}}"#,
        )
        .unwrap();
        let cfg = crate::config::resolve(None, input.tenant_overlay.as_ref()).unwrap();
        assert_eq!(cfg.collection, "", "nothing named a collection");

        let host = happy_host(1);
        let err = search(&host, &cfg, &input).unwrap_err();
        let RagError::InvalidInput(msg) = err else {
            panic!("expected InvalidInput, got {err:?}");
        };
        assert!(msg.contains("admin console"), "message was: {msg}");
        assert!(host.http.calls().is_empty());
    }
}
