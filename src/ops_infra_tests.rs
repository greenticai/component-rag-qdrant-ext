//! Tenant isolation through the connected resource's `collection_prefix`.

use crate::input;
use crate::ops;
use crate::testkit::{cfg_no_legacy, infra_host};

/// Run all six tools once against `host`.
fn run_all(host: &crate::testkit::TestHost, overlay_collection: Option<&str>) {
    let cfg = cfg_no_legacy();
    let ov = overlay_collection
        .map(|c| format!(r#","_tenant_overlay":{{"collection":"{c}"}}"#))
        .unwrap_or_default();
    let j = |body: &str| format!("{{{body}{ov}}}");

    ops::search(
        host,
        &cfg,
        &input::parse_search(&j(r#""vector":[0.1,0.2,0.3]"#)).unwrap(),
    )
    .unwrap();
    ops::upsert(
        host,
        &cfg,
        &input::parse_upsert(&j(r#""id":"1","vector":[0.1,0.2,0.3]"#)).unwrap(),
    )
    .unwrap();
    ops::ingest(
        host,
        &cfg,
        &input::parse_ingest(&j(r#""doc_id":"d1","text":"short""#)).unwrap(),
    )
    .unwrap();
    ops::delete(
        host,
        &cfg,
        &input::parse_delete(&j(r#""doc_id":"d1""#)).unwrap(),
    )
    .unwrap();
    ops::ensure_collection(host, &cfg, &input::parse_ensure(&j("")).unwrap()).unwrap();
    ops::list(host, &cfg, &input::parse_list(&j("")).unwrap()).unwrap();
}

fn urls(host: &crate::testkit::TestHost) -> Vec<String> {
    host.http
        .calls()
        .into_iter()
        .map(|c| c.url)
        .filter(|u| !u.contains("embeddings"))
        .collect()
}

#[test]
fn all_six_tools_address_the_prefixed_collection() {
    let host = infra_host("acme__", 1);
    run_all(&host, None);
    let urls = urls(&host);
    assert!(urls.len() >= 7, "{urls:?}");
    for u in &urls {
        assert!(u.contains("/collections/acme__kb"), "{u}");
        assert!(!u.contains("/collections/kb"), "{u}");
    }
    // One request per tool kind actually went out.
    for needle in [
        "/points/query",
        "/points?wait=true",
        "/points/delete",
        "/points/scroll",
    ] {
        assert!(urls.iter().any(|u| u.contains(needle)), "no {needle}");
    }
    assert!(
        urls.iter().any(|u| u.ends_with("/collections/acme__kb")),
        "no ensure"
    );
}

#[test]
fn the_overlay_pinned_collection_still_wins_and_is_then_prefixed() {
    let cfg = cfg_no_legacy();
    let host = infra_host("acme__", 1);
    let refused = input::parse_search(
        r#"{"vector":[0.1,0.2,0.3],"collection":"x","_tenant_overlay":{"collection":"team-a"}}"#,
    )
    .unwrap();
    let msg = ops::search(&host, &cfg, &refused).unwrap_err().to_string();
    assert!(
        msg.contains("\"x\"") && msg.contains("team-a") && msg.contains("cannot be overridden"),
        "{msg}"
    );
    assert!(
        host.http.calls().is_empty(),
        "a refused call must not fetch"
    );

    let host = crate::testkit::infra_host("acme__", 1);
    crate::testkit::expect_qdrant(&host.http, crate::testkit::INFRA_BASE, "acme__team-a");
    let ok = input::parse_search(
        r#"{"vector":[0.1,0.2,0.3],"_tenant_overlay":{"collection":"team-a"}}"#,
    )
    .unwrap();
    ops::search(&host, &cfg, &ok).unwrap();
    assert!(urls(&host)[0].contains("/collections/acme__team-a/"));
}

#[test]
fn outputs_echo_the_logical_collection_name() {
    let host = infra_host("acme__", 1);
    let cfg = cfg_no_legacy();
    let out = ops::ensure_collection(&host, &cfg, &input::parse_ensure("{}").unwrap()).unwrap();
    assert_eq!(out["collection"], "kb");
    assert!(!out.to_string().contains("acme__"));
    let out = ops::list(&host, &cfg, &input::parse_list("{}").unwrap()).unwrap();
    assert!(!out.to_string().contains("acme__"));
}

#[test]
fn two_tenants_on_one_instance_never_share_a_physical_name() {
    let (a, b) = (infra_host("a__", 1), infra_host("b__", 1));
    run_all(&a, None);
    run_all(&b, None);
    let (ua, ub) = (urls(&a), urls(&b));
    assert!(ua.iter().all(|u| !ub.contains(u)), "{ua:?} / {ub:?}");
}

#[test]
fn no_tool_lists_collections() {
    let host = infra_host("acme__", 1);
    run_all(&host, None);
    for c in host.http.calls() {
        let path = c.url.split("://").nth(1).unwrap_or("");
        let path = path.split_once('/').map_or("", |(_, p)| p);
        assert!(
            path != "collections" && path != "collections/",
            "{} {}",
            c.method,
            c.url
        );
    }
}

#[test]
fn legacy_source_keeps_unprefixed_names() {
    // The ordinary pre-change fixture: legacy url + key, no infra.
    let http = greentic_extension_sdk_testing::mock_host::MockHttpClient::new();
    let secrets = greentic_extension_sdk_testing::mock_host::MockSecretsBackend::new();
    secrets.set(crate::connection::LEGACY_KEY_REF, "qk");
    secrets.set(ops::EMBEDDING_KEY_REF, "ek");
    crate::testkit::expect_qdrant(&http, "https://c.qdrant.io:6333", "kb");
    let host = crate::testkit::TestHost { http, secrets };
    let mut cfg = cfg_no_legacy();
    cfg.qdrant_url = "https://c.qdrant.io:6333".into();
    ops::search(
        &host,
        &cfg,
        &input::parse_search(r#"{"vector":[0.1,0.2,0.3]}"#).unwrap(),
    )
    .unwrap();
    assert!(urls(&host)[0].contains("/collections/kb/"));
}

#[test]
fn ids_are_independent_of_the_prefix() {
    use crate::qdrant::chunk_point_id;
    // Isolation is the collection, not the id namespace: the ids that go on the
    // wire are the plain document-derived ones, whatever the prefix is.
    let body_of = |prefix: &str| {
        let host = infra_host(prefix, 1);
        ops::ingest(
            &host,
            &cfg_no_legacy(),
            &input::parse_ingest(r#"{"doc_id":"doc-1","text":"short"}"#).unwrap(),
        )
        .unwrap();
        let upsert = host
            .http
            .calls()
            .into_iter()
            .find(|c| c.url.contains("/points?wait=true"))
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&upsert.body.unwrap()).unwrap();
        v["points"][0]["id"].clone()
    };
    assert_eq!(body_of("a__"), body_of("b__"));
    assert_eq!(
        body_of("a__"),
        serde_json::json!(chunk_point_id("doc-1", 0))
    );
}
