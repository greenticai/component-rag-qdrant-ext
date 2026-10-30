//! Error text: it names the resource and never carries a credential.

use greentic_extension_sdk_testing::mock_host::{
    CannedResponse, MockHttpClient, MockSecretsBackend,
};

use crate::error::RagError;
use crate::host::{HostCalls, HttpRequest, HttpResponse};
use crate::input;
use crate::ops;
use crate::testkit::{
    INFRA_BASE, INFRA_CREDENTIAL, TestHost, cfg_no_legacy, infra_doc, infra_host,
};

fn search_args() -> input::SearchInput {
    input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap()
}

fn status(code: u16, body: &str) -> CannedResponse {
    CannedResponse {
        status: code,
        body: body.as_bytes().to_vec(),
    }
}

#[test]
fn a_401_from_qdrant_names_the_kind_and_the_source_without_the_key() {
    let host = infra_host("acme__", 1);
    host.http.expect(
        "POST",
        &format!("{INFRA_BASE}/collections/acme__kb/points/query"),
        status(401, r#"{"error":"bad key S3CRET"}"#),
    );
    let msg = ops::search(&host, &cfg_no_legacy(), &search_args())
        .unwrap_err()
        .to_string();
    assert!(msg.contains("vector-store/qdrant"), "{msg}");
    assert!(
        msg.contains("the connected resource's credential was rejected"),
        "{msg}"
    );
    assert!(!msg.contains(INFRA_CREDENTIAL), "{msg}");

    // Legacy source: names the legacy secret, still no key.
    let http = MockHttpClient::new();
    let secrets = MockSecretsBackend::new();
    secrets.set(crate::connection::LEGACY_KEY_REF, "legacykey");
    http.expect(
        "POST",
        "https://c.qdrant.io:6333/collections/kb/points/query",
        status(403, r#"{"error":"bad key legacykey"}"#),
    );
    let host = TestHost { http, secrets };
    let mut cfg = cfg_no_legacy();
    cfg.qdrant_url = "https://c.qdrant.io:6333".into();
    let msg = ops::search(&host, &cfg, &search_args())
        .unwrap_err()
        .to_string();
    assert!(msg.contains("rag-qdrant/qdrant_api_key"), "{msg}");
    assert!(!msg.contains("legacykey"), "{msg}");
}

#[test]
fn a_qdrant_body_is_truncated_after_the_key_is_scrubbed() {
    let host = infra_host("acme__", 1);
    // The key straddles the 200-character cut if scrubbing came second.
    let body = format!("{}{}tail", "x".repeat(197), INFRA_CREDENTIAL);
    host.http.expect(
        "POST",
        &format!("{INFRA_BASE}/collections/acme__kb/points/query"),
        status(500, &body),
    );
    let msg = ops::search(&host, &cfg_no_legacy(), &search_args())
        .unwrap_err()
        .to_string();
    assert!(!msg.contains("S3C"), "partial key leaked: {msg}");
}

/// Delegates to a real `TestHost` but fails selected calls with text that
/// echoes the credential, the way a transport error echoing a URL would.
struct Echoing {
    inner: TestHost,
    fail_fetch: bool,
    fail_embedding_key: bool,
}

impl HostCalls for Echoing {
    fn fetch(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        if self.fail_fetch {
            return Err(format!("failed GET {}?api-key={INFRA_CREDENTIAL}", req.url));
        }
        self.inner.fetch(req)
    }
    fn secret(&self, uri: &str) -> Result<String, String> {
        if self.fail_embedding_key && uri == ops::EMBEDDING_KEY_REF {
            return Err(format!("backend said {INFRA_CREDENTIAL}"));
        }
        self.inner.secret(uri)
    }
}

#[test]
fn a_transport_error_names_the_kind_and_host_port_not_the_url_query_or_key() {
    let host = Echoing {
        inner: infra_host("acme__", 1),
        fail_fetch: true,
        fail_embedding_key: false,
    };
    let msg = ops::search(&host, &cfg_no_legacy(), &search_args())
        .unwrap_err()
        .to_string();
    assert!(msg.contains("vector-store/qdrant"), "{msg}");
    assert!(msg.contains("q.example.com:6333"), "{msg}");
    assert!(!msg.contains(INFRA_CREDENTIAL), "{msg}");
}

#[test]
fn scrub_helper_is_exercised_on_every_host_error_wrapper() {
    // secret() wrapper
    let host = Echoing {
        inner: infra_host("acme__", 1),
        fail_fetch: false,
        fail_embedding_key: true,
    };
    let q = input::parse_search(r#"{"query":"hello"}"#).unwrap();
    let msg = ops::search(&host, &cfg_no_legacy(), &q)
        .unwrap_err()
        .to_string();
    assert!(!msg.contains(INFRA_CREDENTIAL), "{msg}");
    // send() wrapper on the embeddings call
    let host = Echoing {
        inner: infra_host("acme__", 1),
        fail_fetch: true,
        fail_embedding_key: false,
    };
    let msg = ops::search(&host, &cfg_no_legacy(), &q)
        .unwrap_err()
        .to_string();
    assert!(!msg.contains(INFRA_CREDENTIAL), "{msg}");
}

#[test]
fn every_error_in_the_connection_path_names_the_kind() {
    let bad_port = infra_doc(INFRA_CREDENTIAL, "{}").replace("\"port\":6333", "\"port\":0");
    let docs: Vec<Option<String>> = vec![
        None,                                    // not connected
        Some(infra_doc(INFRA_CREDENTIAL, "{}")), // no prefix
        Some(infra_doc(
            INFRA_CREDENTIAL,
            r#"{"collection_prefix":"a/b"}"#,
        )),
        Some(bad_port),
        Some(infra_doc(INFRA_CREDENTIAL, "{}").replace("\"v\":1", "\"v\":9")),
    ];
    for doc in docs {
        let secrets = MockSecretsBackend::new();
        if let Some(d) = &doc {
            secrets.set(crate::infra::URI, d);
        }
        let host = TestHost {
            http: MockHttpClient::new(),
            secrets,
        };
        let err = ops::search(&host, &cfg_no_legacy(), &search_args()).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("vector-store/qdrant"), "{msg}");
        // Never the document itself.
        assert!(
            !msg.contains("\"credential\"") && !msg.contains(INFRA_CREDENTIAL),
            "{msg}"
        );
        assert!(host.http.calls().is_empty());
        assert!(matches!(err, RagError::InvalidInput(_)), "{err:?}");
    }
}

#[test]
fn a_malformed_2xx_body_is_scrubbed_like_a_failure_body() {
    let host = infra_host("acme__", 1);
    host.http.expect(
        "POST",
        &format!("{INFRA_BASE}/collections/acme__kb/points/query"),
        status(200, "garbage S3CRET"),
    );
    let msg = ops::search(&host, &cfg_no_legacy(), &search_args())
        .unwrap_err()
        .to_string();
    assert!(!msg.contains(INFRA_CREDENTIAL), "{msg}");
    // The scrub is applied to the response itself, whatever the parser quotes.
    let probe = TestHost {
        http: MockHttpClient::new(),
        secrets: MockSecretsBackend::new(),
    };
    probe
        .http
        .expect("GET", "https://h/x", status(200, "a S3CRET b"));
    let req = HttpRequest {
        method: "GET".into(),
        url: "https://h/x".into(),
        headers: vec![],
        body: None,
    };
    let resp = ops::send(&probe, &req, "t", &[INFRA_CREDENTIAL]).unwrap();
    assert_eq!(String::from_utf8(resp.body).unwrap(), "a <redacted> b");
}
