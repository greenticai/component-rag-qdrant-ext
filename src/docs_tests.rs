//! Cheap guards against a stale README / AGENTS.md: the failure mode this
//! repo's history shows twice (docs describing a configuration that no longer
//! is the whole story).

const README: &str = include_str!("../README.md");
const AGENTS: &str = include_str!("../AGENTS.md");
const ARCHITECTURE: &str = include_str!("../docs/ARCHITECTURE.md");
const PROMPT: &str = include_str!("../prompts/system.md");

#[test]
fn readme_does_not_claim_the_old_required_secret() {
    for needle in [
        "Qdrant resource",
        "vector-store/qdrant",
        "collection_prefix",
        "legacy",
        "## Migrating from the legacy Qdrant configuration",
        "REST port",
        "flow node",
        "secret://infra/vector_store_qdrant",
    ] {
        assert!(README.contains(needle), "README.md must mention {needle:?}");
    }
    // The configuration table used to say `qdrant_url` is required.
    assert!(
        !README.contains("| `qdrant_url` | yes |"),
        "qdrant_url is the legacy source now, not a required field"
    );
}

#[test]
fn agents_md_lists_the_new_modules_and_the_precedence() {
    for needle in [
        "src/infra.rs",
        "src/connection.rs",
        "x-requires",
        "infra -> legacy",
    ] {
        assert!(AGENTS.contains(needle), "AGENTS.md must mention {needle:?}");
    }
}

#[test]
fn architecture_and_prompt_describe_the_resource() {
    assert!(ARCHITECTURE.contains("vector-store/qdrant"));
    assert!(ARCHITECTURE.contains("collection_prefix"));
    assert!(
        PROMPT.contains("logical"),
        "prompt must say collection names are logical"
    );
}
