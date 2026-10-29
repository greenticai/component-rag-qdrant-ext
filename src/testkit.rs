//! Shared host-test fixtures: the SDK mocks adapted to `HostCalls`, plus
//! ready-made hosts that serve a connected `vector-store/qdrant` resource.

use greentic_extension_sdk_testing::mock_host::{
    CannedResponse, MockHttpClient, MockSecretsBackend,
};

use crate::config::{ChunkConfig, Config, EmbeddingConfig};
use crate::host::{HostCalls, HttpRequest, HttpResponse};

pub const INFRA_BASE: &str = "https://q.example.com:6333";
pub const INFRA_CREDENTIAL: &str = "S3CRET";
pub const EMBED_URL: &str = "https://api.openai.com/v1/embeddings";
/// The runner's real "not connected" wordings (greentic-aw-runtime
/// `infra_connection.rs`; designer in-process `SecretsError::NotFound`;
/// greentic-runner-host `agent_node.rs` out-of-process serve path).
pub const REAL_NOT_CONNECTED: &str = "infra resource 'vector_store_qdrant' is not connected to this deployment (looked at secret://infra/vector_store_qdrant); add it in workspace settings and deploy again";
pub const REAL_NOT_FOUND: &str = "secret not found: secret://infra/vector_store_qdrant";
pub const REAL_SERVE_PATH: &str = "infra resource request 'secret://infra/vector_store_qdrant' is not available on the out-of-process serve path (it resolves one credential for every tenant); deploy the worker as a unit";
/// What `TestHost` answers for an infra URI nobody set (the deployed-worker
/// wording; the other two are exercised one by one in `connection` tests).
pub const NOT_CONNECTED: &str = REAL_NOT_CONNECTED;

/// Adapts the SDK's mocks to the extension's `HostCalls` trait.
pub struct TestHost {
    pub http: MockHttpClient,
    pub secrets: MockSecretsBackend,
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
        // Like the real host, an infra resource nobody connected reads as
        // "not connected" (which selects the legacy source).
        self.secrets.get(uri).map_err(|e| {
            if uri == crate::infra::URI {
                NOT_CONNECTED.to_string()
            } else {
                e.to_string()
            }
        })
    }
}

/// A config with NO legacy url: the connection must come from the resource.
pub fn cfg_no_legacy() -> Config {
    Config {
        qdrant_url: String::new(),
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

pub fn ok(json: &str) -> CannedResponse {
    CannedResponse {
        status: 200,
        body: json.as_bytes().to_vec(),
    }
}

pub fn embed_ok(n: usize) -> CannedResponse {
    let data: Vec<serde_json::Value> = (0..n)
        .map(|i| serde_json::json!({"index": i, "embedding": [0.1, 0.2, 0.3]}))
        .collect();
    CannedResponse {
        status: 200,
        body: serde_json::json!({ "data": data }).to_string().into_bytes(),
    }
}

pub fn infra_doc(credential: &str, extras: &str) -> String {
    format!(
        r#"{{"v":1,"kind":"vector_store_qdrant","host":"q.example.com","port":6333,
        "scheme":"https","base_url":"{INFRA_BASE}","credential":"{credential}",
        "extras":{extras}}}"#
    )
}

/// Register every Qdrant endpoint for `physical` on `base`.
pub fn expect_qdrant(http: &MockHttpClient, base: &str, physical: &str) {
    let c = format!("{base}/collections/{physical}");
    http.expect("PUT", &c, ok(r#"{"result":true}"#));
    http.expect(
        "PUT",
        &format!("{c}/points?wait=true"),
        ok(r#"{"status":"ok"}"#),
    );
    http.expect(
        "POST",
        &format!("{c}/points/delete?wait=true"),
        ok(r#"{"status":"ok"}"#),
    );
    http.expect(
        "POST",
        &format!("{c}/points/query"),
        ok(r#"{"result":{"points":[{"id":"p1","score":0.9,"payload":{"text":"hi"}}]}}"#),
    );
    http.expect(
        "POST",
        &format!("{c}/points/scroll"),
        ok(r#"{"result":{"points":[{"id":"p1","payload":{"doc_id":"d1","chunk_index":0}}],"next_page_offset":null}}"#),
    );
}

/// A host serving a connected resource with `collection_prefix = prefix`.
pub fn infra_host(prefix: &str, embeddings: usize) -> TestHost {
    let http = MockHttpClient::new();
    let secrets = MockSecretsBackend::new();
    secrets.set(
        crate::infra::URI,
        &infra_doc(
            INFRA_CREDENTIAL,
            &format!(r#"{{"collection_prefix":"{prefix}"}}"#),
        ),
    );
    secrets.set(crate::ops::EMBEDDING_KEY_REF, "ek");
    http.expect("POST", EMBED_URL, embed_ok(embeddings));
    expect_qdrant(&http, INFRA_BASE, &format!("{prefix}kb"));
    TestHost { http, secrets }
}
