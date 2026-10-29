//! One conformance pass over all six tools with a stubbed secrets host.

use greentic_extension_sdk_testing::mock_host::{
    CapturedRequest, MockHttpClient, MockSecretsBackend,
};

use crate::config::Config;
use crate::connection::LEGACY_KEY_REF;
use crate::error::RagError;
use crate::host::{HostCalls, HttpRequest, HttpResponse};
use crate::input;
use crate::ops;
use crate::testkit::{
    INFRA_BASE, INFRA_CREDENTIAL, NOT_CONNECTED, TestHost, cfg_no_legacy, expect_qdrant, infra_doc,
    infra_host,
};

const LEGACY_BASE: &str = "https://c.qdrant.io:6333";

type Tool = fn(&TestHost, &Config) -> Result<serde_json::Value, RagError>;

fn tools() -> Vec<(&'static str, Tool)> {
    vec![
        ("rag_search", |h, c| {
            ops::search(
                h,
                c,
                &input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap(),
            )
        }),
        ("rag_upsert", |h, c| {
            ops::upsert(
                h,
                c,
                &input::parse_upsert(r#"{"id":"1","vector":[0.1,0.2,0.3]}"#).unwrap(),
            )
        }),
        ("rag_ingest", |h, c| {
            ops::ingest(
                h,
                c,
                &input::parse_ingest(r#"{"doc_id":"d","text":"short"}"#).unwrap(),
            )
        }),
        ("rag_delete", |h, c| {
            ops::delete(h, c, &input::parse_delete(r#"{"doc_id":"d"}"#).unwrap())
        }),
        ("rag_collection_ensure", |h, c| {
            ops::ensure_collection(h, c, &input::parse_ensure("{}").unwrap())
        }),
        ("rag_list", |h, c| {
            ops::list(h, c, &input::parse_list("{}").unwrap())
        }),
    ]
}

fn api_key_of(req: &CapturedRequest) -> Option<String> {
    req.headers
        .iter()
        .find(|(k, _)| k == "api-key")
        .map(|(_, v)| v.clone())
}

fn legacy_host() -> TestHost {
    let http = MockHttpClient::new();
    let secrets = MockSecretsBackend::new();
    secrets.set(LEGACY_KEY_REF, "qk");
    secrets.set(ops::EMBEDDING_KEY_REF, "ek");
    http.expect(
        "POST",
        crate::testkit::EMBED_URL,
        crate::testkit::embed_ok(1),
    );
    expect_qdrant(&http, LEGACY_BASE, "kb");
    TestHost { http, secrets }
}

fn qdrant_calls(host: &TestHost) -> Vec<CapturedRequest> {
    host.http
        .calls()
        .into_iter()
        .filter(|c| !c.url.contains("embeddings"))
        .collect()
}

#[test]
fn infra_ok_path_end_to_end() {
    let host = infra_host("acme__", 1);
    let cfg = cfg_no_legacy();
    ops::ingest(
        &host,
        &cfg,
        &input::parse_ingest(r#"{"doc_id":"d","text":"short"}"#).unwrap(),
    )
    .unwrap();
    ops::search(
        &host,
        &cfg,
        &input::parse_search(r#"{"query":"hi"}"#).unwrap(),
    )
    .unwrap();
    ops::delete(
        &host,
        &cfg,
        &input::parse_delete(r#"{"doc_id":"d"}"#).unwrap(),
    )
    .unwrap();
    let calls = qdrant_calls(&host);
    assert!(calls.len() >= 4);
    for c in &calls {
        assert_eq!(
            api_key_of(c).as_deref(),
            Some(INFRA_CREDENTIAL),
            "{}",
            c.url
        );
        assert!(c.url.starts_with(INFRA_BASE), "{}", c.url);
    }
}

#[test]
fn infra_missing_no_legacy_every_tool_fails_before_any_fetch() {
    for (name, run) in tools() {
        let host = TestHost {
            http: MockHttpClient::new(),
            secrets: MockSecretsBackend::new(),
        };
        let err = run(&host, &cfg_no_legacy()).unwrap_err();
        assert!(
            err.to_string().contains("vector-store/qdrant"),
            "{name}: {err}"
        );
        assert!(host.http.calls().is_empty(), "{name} fetched");
    }
}

#[test]
fn infra_missing_legacy_set_every_tool_works_exactly_as_0_5_0() {
    let mut cfg = cfg_no_legacy();
    cfg.qdrant_url = LEGACY_BASE.into();
    for (name, run) in tools() {
        let host = legacy_host();
        run(&host, &cfg).unwrap_or_else(|e| panic!("{name}: {e}"));
        let calls = qdrant_calls(&host);
        assert!(!calls.is_empty(), "{name}");
        for c in calls {
            assert!(
                c.url.starts_with(&format!("{LEGACY_BASE}/collections/kb")),
                "{}",
                c.url
            );
            assert_eq!(api_key_of(&c).as_deref(), Some("qk"));
        }
    }
}

#[test]
fn rotated_credential_is_picked_up_on_the_next_call() {
    let host = infra_host("acme__", 1);
    let cfg = cfg_no_legacy();
    let args = input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap();
    ops::search(&host, &cfg, &args).unwrap();
    host.secrets.set(
        crate::infra::URI,
        &infra_doc("ROTATED", r#"{"collection_prefix":"acme__"}"#),
    );
    ops::search(&host, &cfg, &args).unwrap();
    let keys: Vec<_> = qdrant_calls(&host).iter().filter_map(api_key_of).collect();
    assert_eq!(
        keys,
        vec![INFRA_CREDENTIAL.to_string(), "ROTATED".to_string()]
    );
}

#[test]
fn the_embedding_key_is_still_the_legacy_secret() {
    let host = infra_host("acme__", 1);
    ops::search(
        &host,
        &cfg_no_legacy(),
        &input::parse_search(r#"{"query":"hello"}"#).unwrap(),
    )
    .unwrap();
    let embed = host
        .http
        .calls()
        .into_iter()
        .find(|c| c.url.contains("embeddings"))
        .expect("an embeddings call");
    assert!(
        embed
            .headers
            .iter()
            .any(|(k, v)| k == "authorization" && v == "Bearer ek")
    );
}

/// A host that, like the real one, refuses a URI the describe does not declare.
struct Undeclared;
impl HostCalls for Undeclared {
    fn fetch(&self, _: &HttpRequest) -> Result<HttpResponse, String> {
        panic!("must not fetch")
    }
    fn secret(&self, uri: &str) -> Result<String, String> {
        if uri == crate::infra::URI {
            Err("permission denied: secret not declared in permissions.secrets".into())
        } else {
            Err(NOT_CONNECTED.into())
        }
    }
}

#[test]
fn a_secret_host_that_traps_on_undeclared_uri_surfaces_permission_denied() {
    let args = input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap();
    let err = ops::search(&Undeclared, &cfg_no_legacy(), &args).unwrap_err();
    assert!(matches!(err, RagError::PermissionDenied(_)), "{err:?}");
    let msg = err.to_string();
    assert!(
        msg.contains("vector-store/qdrant") && msg.contains("permissions.secrets"),
        "{msg}"
    );
}
