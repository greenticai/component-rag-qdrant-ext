# Qdrant extension: consume the "infrastructure resource" (Plan C)

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use `- [ ]` checkboxes. This is a plan only: nothing here has been built, run or published.

**Goal:** `greentic.rag-qdrant` stops owning "where is Qdrant and what is its key" and instead declares `vector-store/qdrant` as an infrastructure need, reads the admin-resolved connection through `host.secrets.get("secret://infra/vector_store_qdrant")`, isolates tenants inside a shared instance with the connection's `extras.collection_prefix`, and keeps today's overlay-configured Qdrant working as an explicit, labelled legacy source.

**Architecture:** one new pure module `src/infra.rs` (typed reader of the v1 guest document, local copy of the golden fixture, value-free errors) + one resolver `src/connection.rs` that turns (infra doc | legacy overlay) into a single `Connection { base_url, api_key, collection_prefix, source }` per call; `ops.rs` asks for it once and passes it to the existing request builders. `lib.rs` stays the only file touching WIT. `x-requires` is a root key in each tool's `input_schema` (this extension has NO node types), mirrored in `tool_meta.rs` (the source of truth `describe.json` is checked against).

**Tech Stack:** Rust 2024, `cargo component` (wasm32-wasip2), `serde_json`, `greentic-extension-sdk-testing 1.2.9` (dev), `gtdx` 1.2.19 (lint/validate/publish).

**Spec / contracts:**
- Designer contract `docs/infra-resources-feed-v1.md` and `docs/superpowers/specs/2026-10-28-extra-infrastructure-resources-design.md` (worktree `greentic-designer/extra-infrastructure-manage-deployment`); grammar read from `src/orchestrate/infra_resources/requires.rs`.
- Plan A (admin) `greentic-designer-admin-infra/docs/superpowers/plans/2026-10-29-extra-infrastructure-resources-admin.md`; Plan B (runtime) `greentic-deployer-infra/docs/superpowers/plans/2026-10-29-extra-infrastructure-resources-runtime.md` (guest document, R4, R7, R14, C7).

## Global Constraints (exact values)

| Item | Value |
|---|---|
| Kind id | `vector-store/qdrant` |
| `x-requires` JSON (root of each tool's `input_schema`) | `"x-requires": [{"kind": "vector-store/qdrant"}]` |
| Slot | `contributions.tools[].input_schema` (tool-only extension; the designer reads `/contributions/nodeTypes[].config_schema` AND `/contributions/tools[].input_schema`, `requires.rs::from_describe`). Never a new `contributions` key |
| Allowed `contributions` keys | exactly `tools`, `views` (the ratchet pins this) |
| Guest URI | `secret://infra/vector_store_qdrant` (canonical name of the kind) |
| Guest document (Plan B R5) | `{"v":1,"kind":"vector-store/qdrant","host","port","scheme","base_url","credential","extras"}`; `base_url` IPv6-bracketed; reader accepts absent `v` as 1, refuses `v > 1` by name |
| Isolation key | `extras.collection_prefix` (Plan B R14 / admin plan Task 7) |
| Port semantics | the resource `port` is Qdrant's **REST** port (6333 by default); this extension speaks REST only (`/collections`, `/points/query`, `/points/scroll`) |
| Legacy secrets (kept) | `secret://rag-qdrant/qdrant_api_key`, `secret://rag-qdrant/embedding_api_key` |
| Embedding key | stays `secret://rag-qdrant/embedding_api_key` (no embedding kind exists in the catalogue) |
| Error rule | every message that concerns the resource names the kind `vector-store/qdrant`; no message, `Debug` or log line contains the credential or the document |
| Version | `0.5.0` -> `0.6.0` in `Cargo.toml`, `describe.json` `metadata.version`, `runtime.components.rag-qdrant.gtpack.component_version`; `sha256` fields are written by `gtdx publish`, never by hand |
| Commits | conventional, NO Claude co-author trailer, never `--no-verify`, never push from this plan |
| Local gate | `./ci/local_check.sh` (fmt, clippy `-D warnings`, `cargo test`, `./build.sh`); this extension's `cargo test` is milliseconds and host-only (no WASM runtime, no network), so it is safe to run locally, unlike the designer repo |
| Max file size | keep new files < 500 lines (repo convention) |

### Review Focus
1. **Fail closed.** No path reaches `http://localhost:6333` or any default host. Missing infra with no legacy config = named error; missing `collection_prefix` on the infra source = named error.
2. **Credential hygiene**: grep the diff for `format!`/`tracing`/`{:?}` touching `credential`, `api_key`, or the raw document. serde errors are never forwarded (serde echoes the offending value).
3. **Precedence is explicit and tested both ways** (infra present, infra absent + legacy, both absent, both present and different).
4. **`lib.rs` stays the only WIT-touching file**; new modules take `&impl HostCalls`.
5. **Older designers still load the describe** (ratchet in Task 1).

---

## Rulings (every ambiguity; recommendation + cost if wrong)

**R1 - Declare `x-requires` on all six tools, not a subset.** The designer's gate is per extension (`needs_for_unit` unions every schema), so declaring on one tool already gates the whole extension. Declaring on all six keeps the description honest and survives a future split. *Cost if wrong:* none functional; a reader assumes only some tools need it.

**R2 - Declare it unconditionally (hard requirement), and keep a legacy source for lanes the gate cannot cover.** The gate refuses deploys of units using this extension until a Qdrant resource exists (`infra_resource_unmet`, with Provide). That is the feature. But Run Demo and Test chat have no infra overlay (Plan B R17) and would otherwise break. *Cost if wrong:* an existing tenant with `qdrant_url` set in the overlay is blocked at the next deploy until an operator "Provides" an external resource (one dialog). Mitigation: the CHANGELOG/README migration note and Task 8 copy.

**R3 - Precedence: infra -> legacy overlay -> error. Infra wins when both exist.** Order is the opposite of the old "overlay beats init" rule on purpose: the admin-resolved resource is what the deploy gate verified and what the operator sees on the Resources page; an overlay `qdrant_url` left behind must not silently redirect traffic elsewhere. When infra is `Missing` AND a legacy `qdrant_url` is configured, legacy is used and the call's result is unchanged but the source is recorded in `tracing`-free form (see R4). When infra is `Missing` and nothing legacy is configured: error naming the kind. When infra is *present but invalid* (`Invalid`, `UnsupportedVersion`, `Unreadable`) the call **fails; it never falls back to legacy** (a corrupt resource must not quietly route to another cluster). *Cost if wrong:* if legacy won, a stale `qdrant_url` could shadow the admin resource; if we fell back on invalid, a broken credential would hide behind a working old cluster.

**R4 - Surface the source without changing output schemas.** No new field in tool outputs (the describe-drift test compares output schemas). The source (`infra`|`legacy`) is exposed only via the error text of failures and a one-line note in `rag_collection_ensure`'s existing `message`/notes field if it has one (UNVERIFIED which field; check `ops.rs` ensure result in Task 4 and use the existing free-text slot, else skip). *Cost if wrong:* operators cannot see which source served a call; acceptable, the Resources page is the source of truth.

**R5 - Legacy collections are NOT prefixed; infra collections ARE.** Physical collection = `collection_prefix + logical` only for the infra source. Switching an existing tenant from legacy to infra therefore points at new (empty) collections; there is **no auto-migration** (copying vectors between clusters is out of scope and unsafe to do implicitly). The migration note says: re-ingest after connecting the resource, or keep the legacy source until you do. *Cost if wrong:* a silent "my knowledge disappeared" report; mitigated by the note and by R3 (legacy still serves until the resource exists).

**R6 - Missing `collection_prefix` fails closed, with one explicit opt-out.** The admin plan serves platform/partnership EXTERNAL resources identically to every tenant (R6 there), so an unprefixed connection can be a shared instance. Default: `MissingExtra("collection_prefix")`. Opt-out: the connection's `extras.isolation == "dedicated"` (set by the party that provides a resource that only one tenant/team uses); then the prefix is empty. This needs the admin/Provide dialog to be able to set that extra: **cross-plan ask X2** (Plan A Task 7 / designer Provide dialog); until it lands, tenants with their own dedicated Qdrant provide a `collection_prefix` instead (any non-empty string, e.g. `kb__`). *Cost if wrong:* if opt-out were a config flag, a tenant on a shared instance could un-isolate itself; if there were no opt-out, a tenant with their own cluster must invent a prefix (cheap).

**R7 - Prefix is applied exactly once, in one function, and a caller can never include it.** `connection::physical_collection(prefix, logical)`; the logical name is validated (`[A-Za-z0-9._-]{1,128}`, no `/`, no leading `__`) so a caller cannot smuggle a prefix-escaping name; the prefix itself is validated the same way and length <= 64. Tool outputs echo the **logical** name, never the physical one. *Cost if wrong:* a name like `other__kb` could address another tenant's namespace if the rule were scattered.

**R8 - The tenant-pinned overlay `collection` rule is unchanged and composes with the prefix.** `collection_of` still picks the logical name (overlay pin > caller arg > baseline) with the same refusals; the prefix is applied after. *Cost if wrong:* behavioural drift in the existing, heavily tested rule.

**R9 - The `qdrant_api_key` secret becomes optional; the infra credential replaces it as the `api-key` header value.** `requiredSecrets[qdrant_api_key].required` -> `false` and its description says "legacy source only". Otherwise the designer's Setup form keeps asking operators for a key the infra source makes irrelevant (UNVERIFIED how Setup treats `required: false` secrets; the `describe.json` secret form is driven by `requiredSecrets` + tool `secret_requirements`, so both change together, and `tool_meta.rs` line ~780 mirrors them). *Cost if wrong:* a spurious required-credential prompt on every deploy.

**R10 - Network allow-list is a blocker for any host outside `*.qdrant.io`.** `runtime.permissions.network` is static (`https://*.qdrant.io/*`, `https://api.openai.com/*`); `http::fetch` to a platform-managed or self-hosted Qdrant host is rejected before it leaves the sandbox. Neither Plan A nor Plan B addresses it (grep of both: no mention). Recommendation: **cross-plan ask X1** - ext-runtime should treat the host of a connected infra resource as implicitly allowed for the extension that declared `x-requires` (host-side, derived from the same document). Interim, extension-side: none is safe (a `https://*/*` wildcard defeats the sandbox). Task 9 is therefore BLOCKED and the plan's acceptance for managed/self-hosted is conditional on X1. *Cost if wrong:* the feature works only for Qdrant Cloud hosts; managed Qdrant fails with a transport error that does not say "allow-list".

**R11 - Port is REST.** The designer's own contract example shows `6334` (gRPC). This extension cannot use gRPC. We trust `port` as given and add no REST-port guessing; the error for an unreachable host names the kind and `host:port` (not a secret) so a wrong port is diagnosable. Cross-plan ask X3: the admin's Qdrant kind documents that `port` is the REST port. *Cost if wrong:* an operator who enters 6334 gets a connection error; a guessed 6333 would hide a misconfiguration.

**R12 - Own the typed parser in this repo.** Plan B C7 says the guest-side parser "belongs in the extension SDK repo (not in these worktrees)". That SDK crate does not exist yet; waiting blocks everything. Recommendation: a ~150-line local `src/infra.rs` pinned to the same golden fixture (copied byte-for-byte, with a header naming the source repo and the sync rule), to be replaced by the SDK helper when it exists. *Cost if wrong:* duplicated parsing logic; the fixture test makes drift loud.

**R13 - Version 0.6.0 (minor), not 1.0.0.** The tool surface, schemas' properties and outputs are unchanged; the new `x-requires` and the optional secret change deploy-time behaviour only. *Cost if wrong:* none; semver is advisory here, but the Store rejects re-publishing a version, so never reuse 0.5.0.

**R14 - Release is its own step and needs the owner's explicit approval.** Tag `v0.6.0` triggers `publish.yml` (Store). The plan stops at "ready to tag". Also never tag before the dependency gates in the table below are satisfied (an x-requires extension published before the designer gate and the runtime reader ship would gate deploys on a resource nothing can deliver). *Cost if wrong:* deploys blocked or silently keyless; cannot unpublish a Store version.

**R15 - Designer bundling.** `greentic.rag-qdrant` is not in the designer's `bundled/manifest.json` (grep 2026-10-29), so there is no bundled refresh step today; the extension reaches designers through the Store and the admin allow-list auto-sync (upgrades only). If it is ever vendored, `scripts/refresh-bundled.sh` + `bundled/publisher-keys.json` apply (the Store re-signs; check the publisher key before the first vendored refresh). UNVERIFIED: whether the admin allow-list auto-installs this id for existing tenants.

**R16 - Flow nodes are out of scope and the extension must say so.** Plan B R4: a `component.exec` flow node cannot read `secret://infra/...`. This extension declares NO node types (AGENTS.md "No flow node types"), and `INTENDED_FLOW_PALETTE` stays empty; the `node_types_must_resolve_to_a_published_flow_component` guard stays. The describe `description` currently says "to both the agentic worker and flow nodes" - the `capabilities: ["flow","agentic_worker"]` on tools means a flow's agent node may call them (not a `component.exec` node); the wording stays but Task 8 adds a sentence. *Cost if wrong:* an author assumes a flow node can use it.

---

## File map

| File | Change |
|---|---|
| `src/infra.rs` | **create** - typed guest-document reader, value-free errors |
| `src/connection.rs` | **create** - source resolution + `physical_collection` |
| `tests/fixtures/infra_connection_v1.json` | **create** - golden document copied from Plan B Task N1 |
| `src/ops.rs` | modify - one `connection()` call per tool; drop direct `QDRANT_KEY_REF` reads |
| `src/config.rs` | modify - `qdrant_url` no longer mandatory at `resolve`; legacy kept |
| `src/error.rs` | modify - add `RagError::Infra` variants only if needed (prefer existing `InvalidInput`/`PermissionDenied`) |
| `src/tool_meta.rs` | modify - `x-requires` in each schema; secret requirement relaxed; new ratchet tests |
| `describe.json` | modify - mirror `tool_meta`; `permissions.secrets`; `requiredSecrets`; `configSchema` copy; version |
| `README.md`, `AGENTS.md`, `docs/ARCHITECTURE.md`, `prompts/system.md`, `i18n/en.json` | docs |
| `Cargo.toml`, `Cargo.lock` | version only |

(`src/lib.rs` is untouched except, if `host.secrets` needs nothing new, not at all: `WitHost::secret` already forwards any URI.)

---

### Task 1: Declare the need and pin the describe shape (ratchet)

**Files:** modify `src/tool_meta.rs`, `describe.json`.

**Interfaces:** consumes the existing `ToolMeta { input_schema_json: &str, ... }`; produces `pub const INFRA_KIND: &str = "vector-store/qdrant";` in `tool_meta.rs`.

- [ ] **Step 1 - failing tests** (in `tool_meta.rs` `mod tests`):
  - `every_tool_declares_the_qdrant_infrastructure_need` - for each of the six tools in `describe.json`, parse `input_schema`; assert `schema["x-requires"] == json!([{"kind":"vector-store/qdrant"}])` **at the root** (not nested under `properties`/`oneOf`).
  - `x_requires_is_read_by_the_designers_own_grammar` - re-implement the 10-line reader locally (`requires_in_schema`: root key `x-requires`, array of `{kind: string}`) and assert it returns exactly `["vector-store/qdrant"]` for every tool, and nothing for a schema without it. (The designer repo is not a dependency; this mirrors `requires.rs` and cites it in a comment.)
  - `describe_json_declares_no_contributions_key_an_older_designer_would_refuse` - `contributions` object keys are a subset of `{"tools","views"}`; exact-equality to `["tools","views"]` today. Failure message names the reason: `Contributions`/`NodeType`/`Tool` are `deny_unknown_fields`, a new key unloads the extension on every older designer.
  - `no_tool_gains_an_unknown_field` - each tool entry's keys are a subset of the keys present at 0.5.0 (`capabilities, description, export, input_schema, name, output_schema, runtime_ref, secret_requirements` plus any already present - read the set from the 0.5.0 `describe.json` and freeze it as a constant).
  - `x_requires_is_valid_json_schema_keyword_use` - the schema with `x-requires` still parses and keeps its `oneOf` (the either-or tools) intact: assert `rag_search`'s `oneOf` has 2 branches.
  - Extend the existing `describe_json_matches_the_tool_metadata_in_this_file` (it already compares schemas as parsed JSON, so adding the key in BOTH places makes it pass; adding in one fails - that is the drift guard).
- [ ] **Step 2 - run, confirm failure:** `cargo test tool_meta` -> the new `x-requires` assertions fail.
- [ ] **Step 3 - implement:** add `"x-requires": [{"kind": "vector-store/qdrant"}]` to the root object of each of the six schema strings in `tool_meta.rs`, regenerate/mirror into `describe.json` (use the existing `print_contributions` `#[ignore]` test to emit the JSON, then paste; never hand-edit sha256).
- [ ] **Step 4 - run:** `cargo test tool_meta`; `gtdx validate && gtdx lint` (UNVERIFIED: that lint accepts an `x-` keyword inside a string-typed schema; the Store schema types `input_schema` as a string, so expected yes).
- [ ] **Step 5 - commit:** `feat(describe): declare the vector-store/qdrant infrastructure need on every tool`

### Task 2: Typed reader for the guest document (pure)

**Files:** create `src/infra.rs`, `tests/fixtures/infra_connection_v1.json`; modify `src/lib.rs` (`mod infra;` only - no bindings use, so allowed).

**Interfaces (signatures only):**
```rust
pub const KIND: &str = "vector-store/qdrant";
pub const URI: &str = "secret://infra/vector_store_qdrant";
pub const SUPPORTED_VERSION: u32 = 1;

pub struct InfraConnection { /* base_url, host, port, scheme, credential (private), extras */ }
impl InfraConnection {
    pub fn parse(bytes: &str) -> Result<Self, InfraError>;
    pub fn base_url(&self) -> &str;
    pub fn credential(&self) -> &str;            // only caller: header builder
    pub fn extra(&self, key: &str) -> Option<&str>;
    pub fn require_extra(&self, key: &str) -> Result<&str, InfraError>; // blank == absent
}
impl core::fmt::Debug for InfraConnection { /* credential "<redacted>" */ }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InfraError {
    Missing,                                   // host said not connected / undeclared
    Unreadable,                                // host error that is not "missing"
    Invalid { field: &'static str },
    UnsupportedVersion { found: u32 },
    MissingExtra { key: String },
}
impl core::fmt::Display for InfraError { /* names KIND, never a value */ }
```
Classification of the host string error into `Missing` vs `Unreadable` is a single function `classify_host_error(msg: &str) -> InfraError` (UNVERIFIED exact text of Plan B's `Backend` error: it contains "is not connected to this deployment"; match on that phrase and treat everything else as `Unreadable`; both are pinned by tests so a Plan B wording change fails loudly).

- [ ] **Step 1 - failing tests** (`#[cfg(test)]` in `infra.rs`, plus fixture):
  - `parses_the_golden_guest_document` - fixture `{"v":1,"kind":"vector-store/qdrant","host":"q.example.com","port":6333,"scheme":"https","base_url":"https://q.example.com:6333","credential":"S3CRET","extras":{"collection_prefix":"acme__"}}` -> `base_url()=="https://q.example.com:6333"`, `extra("collection_prefix")==Some("acme__")`.
  - `an_absent_version_is_read_as_one_and_a_higher_one_is_refused` - `{"v":2,..}` -> `UnsupportedVersion{found:2}`.
  - `base_url_is_taken_from_the_host_document_not_rebuilt` - a doc with an IPv6 host and bracketed `base_url` yields that exact `base_url` (the host already applied the bracket rule).
  - `a_trailing_slash_on_base_url_is_trimmed`.
  - `invalid_documents_name_the_field_and_never_echo_a_value` - table: port as string `"S3CRET-PORT"`, port 0, empty credential, missing `base_url`, extras with a non-string value -> `Invalid{field}`; for each, `to_string()` and `format!("{:?}")` contain no `S3CRET`.
  - `debug_never_shows_the_credential`.
  - `require_extra_treats_blank_as_absent`.
  - `classify_host_error_maps_the_not_connected_phrase_to_missing_and_the_rest_to_unreadable`.
  - `display_names_the_kind` - every variant's message contains `vector-store/qdrant`.
- [ ] **Step 2:** `cargo test infra::` fails (module absent). **Step 3:** implement (private `RawDoc`, map serde errors to `Invalid{field:"document"}`; never `format!("{e}")`). **Step 4:** `cargo test infra::`; `cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5 - commit:** `feat(infra): typed reader for the connected Qdrant resource`

### Task 3: Connection source resolution (infra -> legacy -> error)

**Files:** create `src/connection.rs`; modify `src/config.rs`, `src/host.rs` (none expected), `src/ops.rs`.

**Interfaces:**
```rust
pub enum Source { Infra, Legacy }
pub struct Connection {
    pub base_url: String,
    pub api_key: String,                // private to header builders; no Debug derive that prints it
    pub collection_prefix: Option<String>, // Some only for Source::Infra
    pub source: Source,
}
pub fn resolve_connection<H: HostCalls>(
    host: &H,
    cfg: &Config,                       // legacy qdrant_url may be empty now
) -> Result<Connection, RagError>;
pub fn physical_collection(prefix: Option<&str>, logical: &str) -> Result<String, RagError>;
```
`config::resolve` change: a blank `qdrant_url` is no longer an error there (it becomes `""`); the "no `qdrant_url`" error moves into `resolve_connection`, raised only when infra is `Missing` AND legacy is blank. The existing cross-cluster refusal (`resolve_qdrant_url` init-vs-overlay mismatch) stays for the legacy source only.

- [ ] **Step 1 - failing tests** (`connection.rs`, using a `StubHost` implementing `HostCalls` with a `HashMap<uri, Result<String,String>>` and an ask-log; reuse the shape of `ops.rs`'s existing `MockSecretsBackend`):
  - `infra_present_wins_over_a_configured_legacy_url` - both set, different hosts -> `source==Infra`, `base_url` is the infra one, and the legacy key URI was **never asked** (ask-log excludes `secret://rag-qdrant/qdrant_api_key`).
  - `infra_missing_and_legacy_configured_uses_legacy` - host returns the not-connected phrase; `cfg.qdrant_url` set -> `Source::Legacy`, key read from `secret://rag-qdrant/qdrant_api_key`, `collection_prefix == None`.
  - `infra_missing_and_nothing_legacy_is_a_named_error_not_localhost` - error text contains `vector-store/qdrant` and the words "Add it in workspace settings"; asserts no `localhost`/`127.0.0.1` appears and no fetch was made.
  - `infra_invalid_never_falls_back_to_legacy` - host returns a doc with port 0 while legacy is configured -> `Err`, legacy key never asked.
  - `infra_unreadable_never_falls_back_to_legacy` - host returns some other error -> `Err`.
  - `infra_unsupported_version_is_refused_by_name`.
  - `a_missing_collection_prefix_fails_closed` - infra doc without extras -> `Err` naming `collection_prefix` and the kind; with `extras.isolation=="dedicated"` and no prefix -> `Ok`, `collection_prefix == Some("")`... represented as `None` plus source Infra (assert the physical name equals the logical name).
  - `a_blank_prefix_is_treated_as_missing_unless_dedicated`.
  - `physical_collection_applies_the_prefix_once` - `("acme__","kb") -> "acme__kb"`; `(None,"kb") -> "kb"`.
  - `physical_collection_refuses_names_that_could_escape_the_namespace` - `""`, `"a/b"`, `"../x"`, 129 chars, a name that already starts with the prefix is **allowed** (it is just a name; physical becomes `acme__acme__kb`) - assert that, so nobody "dedupes" it into a cross-tenant hole.
  - `prefix_rules` - prefix > 64 chars or containing `/` -> `Err` naming `collection_prefix`.
  - `the_credential_is_not_in_any_error_text` - run every failing case above with credential `S3CRET`; assert no error string contains it.
- [ ] **Step 2:** fails (module absent). **Step 3:** implement; `ops.rs` replaces its six `secret(host, QDRANT_KEY_REF)` + `cfg.qdrant_url` uses with one `let conn = resolve_connection(host, &cfg)?;` placed **before** any `collection_of`/HTTP work (so a misconfigured resource fails first and cheaply). Keep `QDRANT_KEY_REF` (legacy). **Step 4:** `cargo test` (the whole existing suite must stay green - it is the regression net for `collection_of`). **Step 5 - commit:** `feat(config): read the Qdrant connection from the infrastructure resource, legacy as explicit fallback`

### Task 4: Tenant isolation through `collection_prefix`

**Files:** modify `src/ops.rs` (apply `physical_collection` after `collection_of` in each of the six tools), `src/qdrant.rs` only if a builder echoes the collection (outputs must echo the logical name).

- [ ] **Step 1 - failing tests** (`ops.rs` tests with `StubHost` serving an infra doc whose `collection_prefix` is `acme__`, and a recording `fetch`):
  - `all_six_tools_address_the_prefixed_collection` - table over `rag_search, rag_upsert, rag_ingest, rag_delete, rag_collection_ensure, rag_list`: every recorded request URL contains `/collections/acme__kb` and none contains `/collections/kb/`.
  - `the_overlay_pinned_collection_still_wins_and_is_then_prefixed` - overlay `collection:"team-a"` + caller arg `collection` -> still refused (existing message); without the arg -> `/collections/acme__team-a`.
  - `outputs_echo_the_logical_collection_name` - `rag_collection_ensure` / `rag_list` results contain `"kb"` and not `acme__kb`.
  - `two_tenants_on_one_instance_never_share_a_physical_name` - two stub hosts with prefixes `a__` and `b__`, same logical `kb`: URL sets are disjoint.
  - `no_tool_lists_collections` - assert across all recorded requests no `GET /collections` (without a name) is ever issued, so a shared instance cannot enumerate other tenants' collections.
  - `legacy_source_keeps_unprefixed_names` (regression for R5).
  - `ids_are_independent_of_the_prefix` - `chunk_point_id("doc-1",0)` identical under both prefixes (the UUIDv5 namespace is not collection-scoped; isolation is the collection).
- [ ] **Step 2-4:** run `cargo test ops::`, implement, run again. **Step 5 - commit:** `feat(ops): isolate tenants in a shared Qdrant by collection prefix`

### Task 5: Error messages and redaction audit

**Files:** modify `src/qdrant.rs` (the `Qdrant rejected the api-key (HTTP {status}): {text}` line), `src/error.rs` if needed, `src/ops.rs`.

- [ ] **Step 1 - failing tests:**
  - `a_401_from_qdrant_names_the_kind_and_the_source_without_the_key` - infra source: message contains `vector-store/qdrant` and "the connected resource's credential was rejected"; legacy: contains `rag-qdrant/qdrant_api_key`; neither contains the key value, **even when the stubbed Qdrant body echoes it** (stub body `{"error":"bad key S3CRET"}`): the response text from Qdrant is truncated to 200 chars **and** scrubbed of the credential string (replace every occurrence with `<redacted>`).
  - `a_transport_error_names_the_kind_and_host_port_not_the_url_query_or_key` - stub `fetch` error containing the key (`"failed GET https://h:6333/?api-key=S3CRET"`) -> message has no `S3CRET`.
  - `every_error_in_the_connection_path_names_the_kind` - table of Task 3 failures: `contains("vector-store/qdrant")`.
  - `no_error_message_contains_the_full_infra_document` - assert the base_url's host may appear but `"credential"` key text never does.
  - `scrub_helper_is_exercised_on_every_host_error_wrapper` - `send()` and `secret()` wrappers in `ops.rs` pass messages through the scrubber (test with the credential as the substring).
- [ ] **Step 2-4:** implement a `redact(msg: &str, secret: &str) -> String` in `connection.rs` (no-op for empty secret; replaces all occurrences), apply wherever a host/Qdrant string is wrapped into `RagError`. **Step 5 - commit:** `fix(errors): name the Qdrant resource and keep the credential out of every message`

### Task 6: describe.json secrets, permissions and configSchema

**Files:** modify `src/tool_meta.rs`, `describe.json`.

- [ ] **Step 1 - failing tests:**
  - `describe_json_permits_the_infra_secret_uri_verbatim` - `runtime.permissions.secrets` contains `secret://infra/vector_store_qdrant` **and** both legacy URIs; and equals `infra::URI` (a `const` used by `connection.rs`, so the string cannot drift from the code).
  - `ops_secret_uris_are_all_declared` - every URI constant used in `ops.rs`/`infra.rs` appears in `permissions.secrets` (extend an existing test if present).
  - `the_qdrant_api_key_is_no_longer_required` - `requiredSecrets` entry `rag-qdrant/qdrant_api_key` has `required:false`; the embedding key stays `true`; per-tool `secret_requirements` mirror it (the existing drift test enforces the mirror).
  - `config_schema_marks_qdrant_url_as_legacy` - `configSchema.properties.qdrant_url.description` contains "legacy" and "ignored when a Qdrant resource is connected"; `qdrant_url` is not in `required`. Extend `describe_json_config_schema_parses_and_names_the_required_fields` accordingly.
- [ ] **Step 2-4:** edit, `cargo test`, `gtdx validate && gtdx lint`. **Step 5 - commit:** `feat(describe): permit the infra secret, relax the legacy key, mark qdrant_url legacy`

### Task 7: Conformance test over all tools with a stubbed secrets host

**Files:** modify `src/ops.rs` tests (or create `src/ops_infra_tests.rs`, `#[cfg(test)] mod` in `lib.rs`).

- [ ] **Step 1 - tests** (each parameterised over the six tools via the existing `dispatch`-level helper or `ops::*` fns):
  - `infra_ok_path_end_to_end` - ingest then search then delete against stubbed Qdrant; asserts `api-key` header equals the infra credential on every request and `base_url` host equals the infra host.
  - `infra_missing_no_legacy_every_tool_fails_before_any_fetch` - six tools, zero `fetch` calls, error names the kind.
  - `infra_missing_legacy_set_every_tool_works_exactly_as_0_5_0` - golden behaviour: URLs and headers byte-identical to the pre-change expectations (reuse existing fixtures).
  - `rotated_credential_is_picked_up_on_the_next_call` - stub returns credential A then B across two calls; second request carries B (no caching in the guest, Plan B R15).
  - `the_embedding_key_is_still_the_legacy_secret` - infra source does not change `embedding_api_key` resolution.
  - `a_secret_host_that_traps_on_undeclared_uri_surfaces_permission_denied` - stub returns the permission error for `secret://infra/...` (describe not updated) -> error names the kind and says the extension's `permissions.secrets` must declare it.
- [ ] **Step 2-4:** run `cargo test`, then `./ci/local_check.sh` (full gate). **Step 5 - commit:** `test(infra): pin the connection paths for every tool against a stubbed secrets host`

### Task 8: Docs, prompt and copy

**Files:** `README.md` (Secrets, Configuration, Collections and tenancy, Requirements and limits), `AGENTS.md` (layout list: `infra.rs`, `connection.rs`; add the "Infrastructure resource" section: x-requires slot, ratchet, precedence), `docs/ARCHITECTURE.md` (config + tenancy sections, line ~292 redirect rule), `prompts/system.md`, `i18n/en.json` (only strings that mention configuring a Qdrant URL), `CLAUDE.md` unchanged.

- [ ] **Step 1 - test first:** `readme_does_not_claim_the_old_required_secret` - a doc-grep test (`include_str!("../README.md")`) asserting the strings `Qdrant resource`, `collection_prefix`, `legacy`, and the migration heading are present, and that "set `qdrant_url`" is no longer the only instruction (cheap guard against a stale README, the failure mode this repo's history shows twice).
- [ ] **Step 2 - write:** migration note ("Existing installs keep working: until a Qdrant resource is connected the overlay `qdrant_url` + `rag-qdrant/qdrant_api_key` are used. After connecting one, collections are `collection_prefix + name`; re-ingest. Remove the old key afterwards."), the precedence table, the isolation explanation, the REST-port requirement (R11), the flow-node limitation (R16), the allow-list caveat (R10), UNVERIFIED markers for anything not run.
- [ ] **Step 3:** `cargo test` (the README guard), `./ci/local_check.sh`. **Step 4 - commit:** `docs: connect Qdrant as an infrastructure resource, migration and isolation`

### Task 9: Network allow-list for resource hosts (BLOCKED on X1)

**Files:** `describe.json` `runtime.permissions.network`, README requirements section.

- [ ] **Do not widen to `https://*/*`.** If X1 (ext-runtime implicitly allows the host of a connected infra resource for the declaring extension) lands, this task is docs-only: describe that `network` no longer needs per-host edits for the resource and keep `*.qdrant.io`/`api.openai.com` for the legacy and embedding paths. If X1 is rejected, the fallback is a platform-known managed-host pattern from Plan A's provisioner (UNVERIFIED: its domain is not defined in either plan) added as one extra `https://*.<managed-domain>/*` entry; external hosts outside `*.qdrant.io` then remain unsupported and the Resources-page Provide copy must say so.
- [ ] **Test:** `network_allowlist_has_no_catch_all` - no entry equals `*`, `https://*`, `https://*/*`, or `http://*`.
- [ ] **Commit:** `feat(describe): allow the managed Qdrant host` (only once the host is known).

### Task 10: Version bump and release readiness (its own step; owner approval required)

**Files:** `Cargo.toml`, `Cargo.lock` (`component-rag-qdrant-ext` entry), `describe.json` (`metadata.version`, `runtime.components.rag-qdrant.gtpack.component_version`), README version mentions, `AGENTS.md` header line.

- [ ] **Step 0: STOP and ask the owner** before this task and before tagging (R14). Confirm the dependency gates below are green.
- [ ] **Step 1 - test:** `versions_agree_across_cargo_and_describe` - `env!("CARGO_PKG_VERSION") == describe.metadata.version == component_version`, and `== "0.6.0"` (update the literal in the same commit; the test exists to stop the partial bump the AGENTS.md "sha256 drifts" warning describes).
- [ ] **Step 2:** bump, `./ci/local_check.sh`, `gtdx lint --publish` (expects real sha256; run only in the publish workflow environment - UNVERIFIED locally).
- [ ] **Step 3:** commit `chore(release): 0.6.0` on a PR branch; after merge and owner approval push tag `v0.6.0` (`publish.yml`: check -> `gtdx publish`; the signing key authenticates the upload, the Store re-signs, so publisher-key TOFU for operators is unaffected by who signs - verify with `gtdx verify` against the served artifact, UNVERIFIED).
- [ ] **Step 4 - chain after publish (not in this repo):** (a) Store lists 0.6.0; (b) admin allow-list auto-sync installs upgrades only for tenants that already have some version (designer `store_reconcile::new_ids`); (c) designer bundled refresh applies only if the id is ever vendored (R15); (d) record the release in the designer's end-of-day bump only if bundled.

---

## Dependency table

| Task | Needs Plan A (admin feed/provisioners) | Needs Plan B runner `infra_connection` + `StoreToolSecretsBackend` branch | Needs Plan B deployer/start | Designer pins/gate | Can land first? |
|---|---|---|---|---|---|
| 1 declare + ratchet | no | no | no | merged #3841 already parses it | **yes** (but do NOT release; see R14) |
| 2 typed reader | no | no (golden fixture only) | no | no | **yes** |
| 3 resolution | no | no (stubbed host) | no | no | **yes** |
| 4 isolation | `collection_prefix` key name agreed (C3) | no | no | no | **yes** (code); real prefix values need A Task 7 |
| 5 errors | no | needs the exact "not connected" phrase (R7) - pinned by a test | no | no | **yes** |
| 6 describe secrets/permissions | no | `permissions.secrets` gate shape (R4) | no | no | **yes** |
| 7 conformance | no | no | no | no | **yes** |
| 8 docs | no | no | no | no | **yes** |
| 9 allow-list | host of managed instance (A provisioner) + X1 | X1 in ext-runtime | no | no | **blocked** |
| 10 release | A live (feed served, else every deploy is `infra_resource_admin_unsupported`) | published runner | deployer `infra_secret_category` + start `infra-connections` published | designer `Dockerfile.tools` pins moved, C1 gate shipped | **blocked** - until all upstream publishes land |

Real-world acceptance (UNVERIFIED, requires everything): provide an external Qdrant resource in the designer -> deploy a unit using this extension on Cloud Run -> `rag_ingest` + `rag_search` succeed and the physical collection is `<prefix><name>`; a second tenant cannot see it.

## Cross-plan asks

- **X1** (ext-runtime / Plan B follow-up): allow the host of a connected infra resource for the declaring extension (R10).
- **X2** (Plan A / designer Provide dialog): ability to set `extras.isolation=dedicated` or a `collection_prefix` on a tenant-provided external resource (R6).
- **X3** (Plan A): document that the Qdrant kind's `port` is the REST port (R11).
- **X4** (Plan B C7): when the SDK guest helper exists, replace `src/infra.rs` and keep the fixture test (R12).

## UNVERIFIED register
- `gtdx lint`/Store schema accept `x-requires` inside the string-typed `input_schema` (expected yes).
- Exact wording of the Plan B missing-entry error (pinned by a test, changes will fail it).
- Designer Setup behaviour for `required:false` secrets.
- Whether `rag_collection_ensure` has a free-text field to carry the source note (R4).
- Whether the admin auto-installs this id for existing tenants; whether it is ever vendored.
- Everything end-to-end (no live admin, runner, deployer, Cloud Run or cluster was run for this plan).
