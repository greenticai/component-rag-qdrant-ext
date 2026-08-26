//! OpenAI-shaped embeddings client. Pure: builds a request, parses a response.

use serde::Deserialize;

use crate::config::EmbeddingConfig;
use crate::error::RagError;
use crate::host::HttpRequest;

#[derive(Deserialize)]
struct EmbeddingItem {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbeddingsBody {
    data: Vec<EmbeddingItem>,
}

/// Build the `POST {base_url}/embeddings` request for a batch of inputs.
#[must_use]
pub fn embed_request(cfg: &EmbeddingConfig, inputs: &[String], api_key: &str) -> HttpRequest {
    let body = serde_json::json!({ "model": cfg.model, "input": inputs });
    HttpRequest {
        method: "POST".to_string(),
        url: format!("{}/embeddings", cfg.base_url),
        headers: vec![
            ("authorization".to_string(), format!("Bearer {api_key}")),
            ("content-type".to_string(), "application/json".to_string()),
        ],
        // `to_vec` on a Value we just built cannot fail; fall back to an empty
        // body rather than unwrap, so a future edit can never trap here.
        body: Some(serde_json::to_vec(&body).unwrap_or_default()),
    }
}

/// Parse an embeddings response into vectors ordered by the API's `index`.
///
/// # Errors
/// [`RagError::PermissionDenied`] on 401/403, [`RagError::SchemaInvalid`] when a
/// returned vector's length is not `expected_dim`, [`RagError::Internal`] for any
/// other non-2xx status or an unparseable body.
pub fn parse_embed_response(
    status: u16,
    body: &[u8],
    expected_dim: u32,
) -> Result<Vec<Vec<f32>>, RagError> {
    let text = String::from_utf8_lossy(body);
    if status == 401 || status == 403 {
        return Err(RagError::PermissionDenied(format!(
            "embeddings API rejected the key (HTTP {status}): {text}"
        )));
    }
    if !(200..300).contains(&status) {
        return Err(RagError::Internal(format!(
            "embeddings API returned HTTP {status}: {text}"
        )));
    }

    let parsed: EmbeddingsBody = serde_json::from_slice(body)
        .map_err(|e| RagError::Internal(format!("embeddings response is not JSON: {e}")))?;

    let mut items = parsed.data;
    items.sort_by_key(|item| item.index);

    let expected = expected_dim as usize;
    for item in &items {
        if item.embedding.len() != expected {
            return Err(RagError::SchemaInvalid(format!(
                "embedding at index {} has {} dimensions, collection expects {expected}",
                item.index,
                item.embedding.len()
            )));
        }
    }
    Ok(items.into_iter().map(|item| item.embedding).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::EmbeddingConfig;

    fn cfg() -> EmbeddingConfig {
        EmbeddingConfig {
            base_url: "https://api.openai.com/v1".to_string(),
            model: "text-embedding-3-small".to_string(),
            dimensions: 3,
        }
    }

    #[test]
    fn the_request_targets_the_embeddings_endpoint_with_bearer_auth() {
        let req = embed_request(&cfg(), &["hello".to_string()], "sk-test");
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "https://api.openai.com/v1/embeddings");
        assert!(
            req.headers
                .contains(&("authorization".to_string(), "Bearer sk-test".to_string()))
        );
        assert!(
            req.headers
                .contains(&("content-type".to_string(), "application/json".to_string()))
        );
    }

    #[test]
    fn the_request_body_carries_the_model_and_every_input() {
        let req = embed_request(&cfg(), &["a".to_string(), "b".to_string()], "k");
        let body: serde_json::Value = serde_json::from_slice(req.body.as_deref().unwrap()).unwrap();
        assert_eq!(body["model"], "text-embedding-3-small");
        assert_eq!(body["input"], serde_json::json!(["a", "b"]));
    }

    #[test]
    fn a_successful_response_yields_vectors_in_index_order() {
        // Deliberately out of order — the API does not promise ordering.
        let body = br#"{"data":[
            {"index":1,"embedding":[0.4,0.5,0.6]},
            {"index":0,"embedding":[0.1,0.2,0.3]}
        ]}"#;
        let got = parse_embed_response(200, body, 3).unwrap();
        assert_eq!(got, vec![vec![0.1, 0.2, 0.3], vec![0.4, 0.5, 0.6]]);
    }

    #[test]
    fn a_dimension_mismatch_is_schema_invalid_not_internal() {
        let body = br#"{"data":[{"index":0,"embedding":[0.1,0.2]}]}"#;
        let err = parse_embed_response(200, body, 3).unwrap_err();
        assert!(matches!(err, RagError::SchemaInvalid(_)), "got {err:?}");
    }

    #[test]
    fn a_401_is_permission_denied() {
        let err = parse_embed_response(401, b"{\"error\":\"bad key\"}", 3).unwrap_err();
        assert!(matches!(err, RagError::PermissionDenied(_)), "got {err:?}");
    }

    #[test]
    fn a_500_is_internal() {
        let err = parse_embed_response(500, b"upstream on fire", 3).unwrap_err();
        assert!(matches!(err, RagError::Internal(_)), "got {err:?}");
    }

    #[test]
    fn a_200_with_a_non_json_body_is_internal_not_a_panic() {
        let err = parse_embed_response(200, b"<html>nope</html>", 3).unwrap_err();
        assert!(matches!(err, RagError::Internal(_)), "got {err:?}");
    }
}
