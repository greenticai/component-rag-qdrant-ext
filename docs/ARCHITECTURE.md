# Architecture

This is the inside view: how this extension was built, and what every tracked file is
for. [`README.md`](../README.md) covers usage — quick start, configuration, the tool
reference, a worked example, and the reasoning behind four design decisions. Read that
first if you want to *call* this extension. Read this one if you want to *build your
own* extension shaped like it.

All facts below were checked against the code at the commit this document was written
against (`v0.3.0`, 115 tests passing). Where a claim could not be checked against
something in this repository, that is said explicitly rather than assumed.

## The one rule, before anything else

**Only `src/lib.rs` may import `crate::bindings`.**

This is forced, not a style preference. This extension compiles to `wasm32-wasip2`; its
host calls (`http::fetch`, `secrets::get`, …) are WIT imports satisfied by the runtime
that loads the component. Call one of those imports from a plain host `cargo test` — no
WASM runtime underneath — and the process **aborts with `SIGABRT`**: non-unwinding,
uncatchable, nothing to `Result` or even `catch_unwind` on, and it takes the entire test
binary down with it, not just the one test.

So every host call is injected behind the `HostCalls` trait (`src/host.rs`): `fetch` and
`secret`. `src/lib.rs`'s `WitHost` is the only implementation backed by real WIT
bindings, and it is also the only place in the crate allowed to import `bindings::`.
Every other module — `ops.rs`, `qdrant.rs`, `embed.rs`, `chunk.rs`, `config.rs`,
`input.rs`, `error.rs`, `tool_meta.rs` — takes `&impl HostCalls` generically, and tests
substitute `greentic-extension-sdk-testing`'s `MockHttpClient` / `MockSecretsBackend`
instead of a real transport. That is what makes 115 tests run in milliseconds on the
host instead of requiring a WASM runtime or a live Qdrant cluster.

If you copy this layout for a new extension: a `bindings::` call in a pure module passes
`cargo check` without complaint, and then `SIGABRT`s the instant a test reaches it — a
failure mode with no useful backtrace. Get this right before writing your first tool.

## Building one yourself

Commands below are taken from `AGENTS.md`, the two build scripts, and `gtdx --help` /
`gtdx new --help` run against the `gtdx` binary in this environment (`1.2.7`) — nothing
here is invented.

1. **Scaffold.** `gtdx new <name> --kind design --id <reverse-dns-id>` (interactive
   wizard if run with no name on a terminal). This produces the skeleton this whole
   document describes: `describe.json` with sample metadata and one placeholder
   tool/node, `wit/world.wit` plus a vendored, locked `wit/deps/`, a starter
   `src/lib.rs`, `Cargo.toml`, `i18n/en.json`, `prompts/system.md`, `build.sh`,
   `ci/local_check.sh`, `rust-toolchain.toml`, `.gitignore`, and the `AGENTS.md` /
   `CLAUDE.md` / `.claude/` agent-config trio. **Easy to get wrong:** the id namespace
   and its WIT-package form must stay in sync in four places for the life of the
   project — `describe.json`'s `metadata.id` and `runtime.components` key,
   `Cargo.toml`'s `package.metadata.component.package`, and `wit/world.wit`'s `package`
   line. Renaming later means touching all four together.

2. **Design the tool surface before writing Rust.** Decide the tool names, JSON
   schemas, and agentic-worker metadata (`side_effects`, `cost`,
   `confirmation_required`, `usage_hint`) up front — they live in one file,
   `src/tool_meta.rs`, as `&'static str` constants plus a `Vec<ToolMeta>` returned by
   `all_tools()`. **Easy to get wrong:** a schema encoding a mutual-exclusion rule (a
   `oneOf` over two fields) is not enough on its own — this extension also enforces it
   a second time in `src/input.rs` at parse time, because a model that reads only
   `required`/`properties` and skips `oneOf` will keep constructing invalid calls
   otherwise.

3. **Write the pure modules.** `error.rs` (the error taxonomy), `host.rs` (the
   `HostCalls` seam), `config.rs` (operator config, parsed once), then the
   domain-specific pure modules — here `chunk.rs`, `embed.rs`, `qdrant.rs` — and
   finally `ops.rs`, which is the only module allowed to sequence more than one host
   call per tool. None of these touch `bindings::`; all of them take `&impl HostCalls`
   or nothing host-shaped at all. This is where nearly all of the logic — and nearly
   all of the tests — live.

4. **Wire `src/lib.rs`.** Implement the WIT-exported `Guest` traits for whatever
   interfaces the world exports (here: `manifest`, `lifecycle`, `tools`, `validation`,
   `prompting`, `knowledge`), map the extension's own error type onto the WIT
   `extension-error` in one place, and dispatch each tool name from `tools::invoke_tool`
   to the matching `ops::*` function. This file should be thin — glue, not logic.

5. **Generate `src/bindings.rs` once.** It does not exist until you build:
   `gtdx dev --once` (or `cargo component build`) generates it from `wit/world.wit` +
   `wit/deps/`. **Easy to get wrong:** `cargo test` on a fresh clone fails with
   `cannot find export in bindings` until this has run at least once — the file is
   gitignored on purpose (see `.gitignore`) and regenerates on every build, so it is
   never something to commit or hand-edit.

6. **Run the fast loop.** `cargo test` — no WASM runtime, milliseconds, this is where
   development actually happens.

7. **Keep the manifest in sync with the code.** `src/tool_meta.rs` is the source of
   truth for `describe.json`'s `contributions.tools`; a host test
   (`describe_json_matches_the_tool_metadata_in_this_file`, see below) asserts they
   have not drifted. When they have, regenerate with
   `RUNTIME_REF=<runtime_ref> cargo test print_contributions -- --ignored --nocapture`
   and paste the printed JSON block into `describe.json` by hand — `print_contributions`
   is a generator, not a check (`#[ignore]`), and there is no command that writes the
   file for you.

8. **Add a contributed view, if the extension has one.** Create
   `assets/views/<view-id>/` with an `index.html` entry, declare it under
   `describe.json`'s `contributions.views[]`, and if the same page must appear on more
   than one host surface, give each surface its own real directory (see
   [The view layer](#the-view-layer) — a symlink lints clean and ships broken).

9. **Run the full gate before every commit.** `./ci/local_check.sh` —
   `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`,
   `cargo test`, then `./build.sh` (`cargo component build --release`). **Known
   scaffold bug, not specific to this extension:** the last step regenerates the
   gitignored `src/bindings.rs`, unformatted, so the script cannot stay green across
   two consecutive runs — its own last step breaks its own first gate on the next
   run. `cargo fmt` before the next commit and move on.

10. **Validate the manifest.** `gtdx doctor` (environment: cargo, cargo-component, the
    `wasm32-wasip2` target), then `gtdx validate` (schema) and `gtdx lint` (cross-field
    invariants — id pattern, schema host, `E_VIEW_REMOTE_ASSET`, …) after any edit to
    `describe.json`.

11. **Install locally and exercise it for real.** `gtdx dev` watches and
    rebuilds/repacks/reinstalls on save; `gtdx dev --once` does it once, producing
    `dist/<id>-<version>.gtxpack` and installing it into `~/.greentic`. This proves the
    package installs — not that it behaves; that is still the `cargo test` layer.

12. **Publish.** `gtdx lint --publish` first (additionally rejects the placeholder
    `0000…` `sha256`), then `gtdx publish`, which builds, computes the real content
    hashes into `describe.json`, packs the `.gtxpack`, and installs it locally.
    `gtdx verify` checks the signature once published/signed. **Easy to get wrong:**
    `gtdx publish` writes the `sha256` fields but never touches
    `runtime.components.*.gtpack.component_version` — bump that by hand, together with
    `Cargo.toml`'s `[package].version` and `describe.json`'s `metadata.version`, on every
    release, or it silently drifts from the real package.

## The manifest layer

**`describe.json`** is what the designer and the store read — the single source of
truth for this extension's identity, capabilities and permissions. It has to stay valid
v2 shape: `apiVersion: greentic.ai/v2`, a `compat` block, and a `runtime.components` map.
Fields split cleanly into authored and generated:

- **Authored by hand:** `metadata.{id,name,summary,description,author,license,
  repository,keywords}`; `compat.{min_designer_version,min_runner_version,
  contract_version}`; `capabilities.{offered,required}` (mirrors the `manifest::Guest`
  impl in `lib.rs`); `runtime.memoryLimitMB`; `runtime.permissions.{network,secrets,
  callExtensionKinds,ui}`; `requiredSecrets`; `contributions.views`.
- **Generated, never hand-edit:** the `sha256` fields under `runtime.components` —
  `gtdx publish` computes the real content hash; until then they are the placeholder
  `0000…0000`, and `gtdx lint --publish` refuses to ship that placeholder
  (`E_SHA256_ZERO`).
- **Generated *from* code, but not by a tool you run automatically:**
  `contributions.tools` is produced from `src/tool_meta.rs` by the `print_contributions`
  test (step 7 above); nothing enforces re-running it except the drift test.
- **Hand-maintained, easy to forget:** `runtime.components.*.gtpack.component_version`.
  Unlike its neighbouring `sha256` field, `gtdx publish` never writes this one, so it
  drifts silently if a release bumps the package version everywhere else.

`runtime.permissions` is the capability gate the running extension is held to:
`network` (an exact-match/wildcard host allowlist any `http::fetch` call is checked
against — here `https://*.qdrant.io/*` and `https://api.openai.com/*`), `secrets` (URIs
the host will resolve — here `secret://rag-qdrant/qdrant_api_key` and
`secret://rag-qdrant/embedding_api_key`, matched verbatim against the same string
literals `ops.rs` uses, `QDRANT_KEY_REF`/`EMBEDDING_KEY_REF`), `callExtensionKinds`
(empty — this extension never calls another extension via the `broker` interface it
imports but does not use for that), and `ui` (see [The view
layer](#the-view-layer) — empty here, for a reason specific to what the contributed view
does and does not need).

The `contributions.tools` ↔ `src/tool_meta.rs` drift check is real and worth reading
once: `tool_meta::tests::describe_json_matches_the_tool_metadata_in_this_file` parses
both `describe.json` (via `include_str!`, so it needs no filesystem access beyond
compile time) and `all_tools()`, and compares `name`, `description`, and the
`input_schema` **as parsed JSON, not as strings** — so reordering keys or changing
whitespace in one copy does not cause a false failure, only an actual schema
disagreement does.

**`.gtdx-contract.lock`** is generated by `gtdx new` and pins the sha256 of every
vendored file under `wit/deps/`. It is a supply-chain lock for the WIT contract, not
project metadata — never edit it, and never edit `wit/deps/` without regenerating it
through the tool that owns it.

**`wit/world.wit`** is this extension's own WIT package, `greentic:rag-qdrant`. It
declares `world extension`, which imports `extension-base/types` plus five
`extension-host` interfaces (`logging`, `i18n`, `secrets`, `broker`, `http`) and
exports five interfaces this extension implements (`extension-base/{manifest,
lifecycle}`, `extension-design/{tools,validation,prompting,knowledge}`). Notably this
is *not* the same as the `design-extension` world defined in
`wit/deps/greentic/extension-design/world.wit`, which additionally imports
`extension-host/llm` and exports a `roles` interface for compiling role DSL entries to
Adaptive Cards / Slack Block Kit / Teams cards — this extension needs neither, so
`src/lib.rs`'s `Component` implements exactly the five interfaces the smaller
`extension` world exports, no more.

**`wit/deps/greentic/{extension-base,extension-design,extension-host}/world.wit`** are
vendored, locked copies of the SDK's shared WIT packages: base identity/lifecycle types,
the design-time interfaces (tools, validation, prompting, knowledge, roles) and worlds,
and the host-provided interfaces (logging, i18n, secrets, broker, http, llm) and their
world. Treat as read-only, exactly as `AGENTS.md` says.

## The Rust guest

Ten tracked `.rs` files under `src/`, plus one generated, gitignored file this section
also has to account for.

| File | Lines | Role |
|---|---:|---|
| `lib.rs` | 313 | WIT export layer |
| `bindings.rs` | 3391 | generated — not tracked |
| `ops.rs` | 1324 | orchestration |
| `qdrant.rs` | 610 | Qdrant REST client |
| `tool_meta.rs` | 458 | tool catalog |
| `input.rs` | 407 | argument parsing |
| `config.rs` | 321 | operator config |
| `embed.rs` | 174 | embeddings client |
| `chunk.rs` | 89 | text splitting |
| `error.rs` | 28 | error taxonomy |
| `host.rs` | 32 | the `HostCalls` seam |

**`src/lib.rs`** — the only module allowed to touch `crate::bindings`. Implements
`Component` against every `Guest` trait the world exports: `manifest::Guest` (static
identity/capabilities), `lifecycle::Guest::init` (parses and stores operator config via
`config::parse_config`/`config::store`; `shutdown` is a no-op — the extension is
stateless, no client or connection pool to drain), `tools::Guest` (`list_tools()` maps
`tool_meta::all_tools()` into the WIT `ToolDefinition` shape; `invoke_tool` calls the
private `dispatch()` function, which parses arguments *before* looking up config, so a
malformed call fails the same way whether or not `init` has run — then routes to the
matching `ops::*` function), `validation::Guest` (always valid — this extension does not
validate designer-authored content), `prompting::Guest` (one static system-prompt
fragment telling an agent to call `rag_search` before answering from stored knowledge),
and `knowledge::Guest` (empty on every method — this extension's content lives in
Qdrant at runtime, reached through `rag_search`, not in the designer's static packaged
knowledge base this interface is for). `WitHost` is the sole `HostCalls` implementation
backed by real bindings. Depends on every other module in the crate; nothing may depend
on it. **Careless-break:** move a `bindings::` call into any other module and it still
compiles — `cargo check` cannot see the problem — then `SIGABRT`s the moment a test
reaches it.

**`src/bindings.rs`** — generated by `cargo component build` (invoked by `build.sh` or
`gtdx dev`/`gtdx publish`) from `wit/world.wit` and `wit/deps/`. Gitignored; never
committed or hand-edited; a fresh clone's first `cargo test` fails with `cannot find
export in bindings` until one build has produced it.

**`src/host.rs`** — the whole host boundary in 32 lines: plain `HttpRequest`/
`HttpResponse` structs and the `HostCalls` trait (`fetch`, `secret`). Depends on
nothing. Everything else in the crate that needs a host call depends on this trait, not
on a concrete implementation. **Careless-break:** none by itself — its entire purpose is
to be the thing that stands between the rest of the crate and `bindings::`.

**`src/error.rs`** — `RagError`, a five-variant enum
(`InvalidInput`/`PermissionDenied`/`NotFound`/`SchemaInvalid`/`Internal`) plus a
`Display` impl. Depends on nothing; every fallible pure function returns this type.
Only `lib.rs::map_error` translates it onto the WIT `extension-error` variant, so no
other module needs `bindings::` just to report an error. **Careless-break:** adding a
new `RagError` variant without a matching arm in `map_error` is a compile error (the
match must stay exhaustive), so this one is close to self-defending — the real risk is
picking the wrong *existing* variant for a new failure (e.g. mapping a config problem to
`Internal` instead of `InvalidInput`), which silently changes how a host is expected to
react to it.

**`src/config.rs`** — `Config`/`EmbeddingConfig`/`ChunkConfig`, deserialized from the
JSON body `lifecycle::init` receives; validates required fields are non-empty, strips
trailing slashes from both URLs (an unstripped one would double a leading-slash path
into `//collections/...`), defaults `chunk.max_chars`/`chunk.overlap_chars` to
1200/150 and `require_tenant_overlay` to `false`. Stored once in a process-wide
`OnceLock` wrapped by `ConfigStore`, whose re-init policy is deliberately asymmetric: a
second `init` call with an *identical* config is treated as a harmless reload and
succeeds; a second call with a *different* config is rejected outright, because a
`OnceLock` cannot be replaced and silently keeping the old config while claiming success
would be a lie. Depends only on `error.rs`. **Careless-break:** relaxing that mismatch
check to "just ignore the second `init`" would let an operator believe a config change
took effect when the running instance is still serving the first one.

**`src/chunk.rs`** — one pure function, `chunk_text`, splitting text into overlapping
character windows for `rag_ingest`. Operates on `Vec<char>`, not bytes, so a multi-byte
UTF-8 character is never split mid-codepoint. The step size is
`max_chars.saturating_sub(overlap_chars).max(1)` — the `.max(1)` clamp is load-bearing:
without it, an operator-misconfigured overlap `>= max_chars` gives a step of zero and
spins forever instead of degrading gracefully. Depends on nothing. **Careless-break:**
removing that clamp reopens the infinite loop for exactly the config `config.rs` already
allows through (it only checks `overlap_chars < max_chars`, not `>= max_chars - 1` or
similar — the guard lives here, not there).

**`src/embed.rs`** — builds the `POST {base_url}/embeddings` request (OpenAI-shaped:
`model` + `input` array, bearer auth) and parses the response into vectors ordered by
the API's own declared `index`. The order check is stricter than "sort by index": it
requires the indices to be exactly the contiguous set `0..n`, because two items both
claiming the same index would sort adjacent and pass a naive length check, silently
pairing one chunk's text with another's vector. Maps 401/403 to `PermissionDenied`, a
dimension mismatch against the configured width to `SchemaInvalid`, everything else
non-2xx or unparseable to `Internal`. Depends on `config::EmbeddingConfig`, `error.rs`,
`host::HttpRequest`. **Careless-break:** relaxing the contiguity check back to a plain
sort-by-index reopens the mispairing bug the test
`a_duplicated_index_is_internal_not_silently_mispaired` exists specifically to catch.

**`src/input.rs`** — one parser per tool (`parse_search`, `parse_upsert`,
`parse_ingest`, `parse_delete`, `parse_ensure`, `parse_list`): JSON in, a validated
typed struct out. Re-enforces every schema's `oneOf` rule in Rust — a model that reads
only `required`/`properties` and skips `oneOf` still gets a clean `InvalidInput` instead
of the extension silently picking a branch. Also validates `rag_upsert`'s `id` is an
unsigned integer or a UUID *before* any host call, because Qdrant would otherwise
reject an unparseable id with an opaque 400 after an embeddings call had already been
spent. Also defines `TenantOverlay`, deserialized from the reserved `_tenant_overlay`
argument key every input struct carries; every field is `Option` and unknown keys are
ignored, so a host that learns to send more of the overlay later cannot break a guest
that has not learned to read it yet. Depends on `error.rs` only. **Careless-break:**
dropping the `exactly_one` check on any of the three either/or tools lets a model send
both fields (or neither) and have the extension guess, instead of failing clearly.

**`src/qdrant.rs`** — pure Qdrant REST request builders (`ensure_collection_request`,
`upsert_request`, `query_request`, `scroll_request`, `delete_request`) and response
parsers (`parse_hits`, `parse_scroll`, `parse_ack`, `parse_ensure_ack`). Owns
`chunk_point_id`, the deterministic UUIDv5 of `"{doc_id}:{chunk_index}"` against a fixed
namespace constant (`CHUNK_NAMESPACE`, the RFC 4122 OID namespace, chosen only because
nothing else in the crate reuses it) — determinism is what lets re-ingesting a document
overwrite its old points instead of accumulating duplicates. `parse_ensure_ack` treats a
4xx body containing "already exists" as success (ensure runs on every ingest/upsert, so
the collection already existing is the normal case), but checks 401/403 *before* that
escape, so an auth failure whose error page happens to echo the phrase back is not
misread as success. Depends on `error.rs`, `host::HttpRequest`. **Careless-break:**
changing `CHUNK_NAMESPACE` orphans every point already written by every install running
an older version — a silent, permanent split between old and new chunk ids for the same
document.

**`src/ops.rs`** — the largest file, and the only one allowed to sequence more than one
host call per tool (`search`, `upsert`, `ingest`, `delete`, `ensure_collection`, `list`,
one public function per tool). Two things live here and nowhere else:

- `collection_of()` — the tenant-isolation precedence: `_tenant_overlay.collection` (if
  present, authoritative) → the caller's `collection` argument (only when no overlay
  pins one) → the operator's configured `collection`. A caller `collection` argument is
  refused outright whenever an overlay pins one, even when it agrees, and every tool
  resolves this *before* embedding, chunking, or any host call, so a refusal never
  follows a billable side effect. See [Tenant isolation](#tenant-isolation) below.
- The `rag_ingest` ordering: chunk → embed → ensure collection → delete the document's
  existing chunks → upsert the new ones. Embedding happens before the delete so a failed
  embeddings call leaves the previous, working version of the document intact; the
  delete happens before the upsert so a document that shrank on re-ingest cannot leave
  orphaned tail chunks that keep matching searches forever. Both orderings are pinned by
  tests, not just prose (`ingest_deletes_the_document_before_upserting_its_chunks`
  asserts positions in the mock's recorded call list).

`rag_list`'s grouping also lives here: `chunk_rank` and `DocGroup` pick one
deterministic winning metadata payload per `doc_id` (lowest `chunk_index`, points
without one ranked last, ties broken by point id string) regardless of the order
Qdrant's scroll happens to return points in, while every point is still counted toward
`chunk_count`. Depends on every pure module. **Careless-break:** reordering `ingest`'s
delete-before-upsert to delete-after (or dropping it) silently orphans stale chunks on
every re-ingest that removes content; moving `collection_of()`'s resolution after
`embed_all` reopens a real regression this repo's own tests were written to close (see
`a_refused_search_on_the_text_path_never_reaches_the_embeddings_api`'s doc comment).

**`src/tool_meta.rs`** — the static `ToolMeta` catalog: one entry per tool with its
name, description, input/output JSON Schema, capability flags (`flow`/
`agentic_worker`), and agentic-worker metadata (`usage_hint`/`side_effects`/`cost`/
`confirmation_required`). This is the Rust source of truth `describe.json`'s
`contributions.tools` is generated from — see [The manifest
layer](#the-manifest-layer) for the drift test that catches disagreement, and the
`print_contributions` generator (`#[ignore]`, run explicitly) that regenerates the
JSON block to paste in by hand. Depends on nothing. **Careless-break:** editing a schema
here without re-running `print_contributions` and pasting the result into
`describe.json` fails exactly one test — `cargo check`, clippy, `gtdx validate`, and
`gtdx lint` all stay green while the designer catalogues a stale copy of the schema.

## The view layer

New in 0.3.0. `contributions.views[]` declares two entries for what is, in every byte
that matters, one page:

```json
{ "id": "knowledge",       "surface": "designer", "min_visibility": "member" }
{ "id": "knowledge-admin", "surface": "admin",     "min_visibility": "tenant_admin" }
```

(elided: `entry: "index.html"`, `placement`, `title_key`/`title_fallback`, and
`tools: ["rag_list","rag_ingest","rag_delete","rag_search"]`, identical on both.) Each
is served from its own directory, `assets/views/knowledge/` and
`assets/views/knowledge-admin/`, and the two are required to be byte-identical — a
test, `view_asset_tests::the_designer_and_admin_copies_of_the_view_are_identical` in
`src/lib.rs`, `include_str!`s all five files from both directories and fails the build
on any diff (`diff -q` against the working tree confirms they are identical right now).

### Why two directories, not one shared bundle

`surface` is single-valued and view ids must be unique across the array (the host
namespaces each into a route, `<extension id>/<view id>`), so one entry cannot cover
both hosts. The asset resolution is the part that is easy to get wrong: `gtdx lint`
resolves `entry` at `assets/views/<view id>/<entry>` and rejects `..` in the path, so
each id needs its own real directory. A symlinked second directory looks like a fix and
is not — lint follows the symlink and passes, the packer copies only real files, and the
resulting `.gtxpack` ships **nothing** under the symlinked id. Lint-clean, install
broken, and nothing before install time tells you. Real duplication, checked by a test,
is the safer failure mode: at worst the two copies drift and the test catches it before
the build finishes.

### What each file does

- **`index.html`** (174 lines) — the shell: a status line, a hidden `#workspace` that
  only appears once `greentic.ready` resolves, two tabs (Documents / Search), a file
  drop zone with a `<input type=file accept=".txt,.text,.csv,.tsv,.md,.markdown,.mdown,
  .mkd,.pdf,...">`, and a toast region. No inline `<script>`; loads `bridge.js`,
  `pdf.js`, then `app.js` in that order.
- **`bridge.js`** (128 lines) — the transport. Wraps `postMessage` into
  `window.greentic.{ready, invokeTool, callApi, fetch, resize, navigate, toast}`. Two
  things worth internalizing if you copy it: it checks `event.source ===
  window.parent`, deliberately **not** `event.origin` — the page runs in
  `sandbox="allow-scripts"` with no `allow-same-origin`, so its origin is the literal
  string `"null"` and a forged message from any other window would look identical by
  that measure; and every call it sends gets a 10-second timeout, after which the
  `Promise` rejects and the slot is freed, whether or not the host is still working on
  it server-side. Marked in `AGENTS.md` and `README.md` as copied byte-for-byte from
  the SDK scaffold and not to be edited — consistent with what is actually here: no
  extension-specific logic in the file at all.
- **`pdf.js`** (1453 lines) — a dependency-free PDF text extractor exposing exactly one
  global, `window.ragPdf.extractPdfText`. Confirmed by reading it: it inflates
  `/FlateDecode` streams with the browser's native `DecompressionStream`, expands
  `/ObjStm` compressed object streams, applies PNG predictors, and resolves glyph→text
  through `/ToUnicode` CMaps first, then `/Differences` encodings, then the standard
  Latin/WinAnsi/MacRoman tables. It rejects rather than guesses: an `/Encrypt` entry (or
  a heuristic fallback scanning for one in plain ASCII trailers) throws before any text
  is read; a page tree with no readable pages throws; and after extraction it computes a
  confidence score (`(1 - badFraction) * plausibleRatio`) and throws if
  `badFraction > 0.02` or `plausibleRatio < 0.6`, warning (not blocking) between that and
  `0.98`. This matches what `AGENTS.md`/`README.md` claim about it, checked directly
  against the source rather than taken on their word.
- **`app.js`** (1093 lines) — the extension-specific logic: staging a file, extracting
  its text in-browser (`TEXT_EXTENSIONS = [txt, text, md, markdown, mdown, mkd, csv,
  tsv]`, plus PDF via `pdf.js`), previewing it, calling `rag_ingest`/`rag_list`/
  `rag_delete`/`rag_search` through the bridge, and rendering results. Confirmed by
  reading it: every value that can trace back to something a caller wrote (a `doc_id`,
  a stored `text`, metadata) is written with `make()`, a small helper around
  `document.createElement` + `textContent` — there is no `innerHTML` anywhere in this
  file, matching the stated rule. There is likewise no `window.confirm`/`window.alert`
  call anywhere in the file (only comments referencing the rule) — delete confirmation
  is done in-page instead, because a native modal would block this frame's event loop
  and, with it, the `message` handler every pending bridge call is waiting on, stranding
  them all until their individual 10-second timeouts fire. `TEXT_EXTENSIONS` here
  includes `csv` and `tsv` alongside the plain-text/Markdown extensions, matching
  `README.md`'s file-format table.
- **`style.css`** (420 lines) — theme tokens on `:root`, redefined under
  `@media (prefers-color-scheme: dark)` guarded by `:root:not([data-theme="light"])`,
  and again under `:root[data-theme="dark"]` so an explicit host-provided theme (from
  the `init` message's `theme` field, applied in `app.js`) wins over the media query in
  both directions. No logic.

### What `runtime.permissions.ui` gates, and why it is empty here

`ui` in `runtime.permissions` grants two things a contributed view can use:
`fetchHosts` (a server-side-proxied `greentic.fetch`, so the page's own opaque-origin
`fetch()` — which would send `Origin: null` and reach nothing useful anyway — is never
needed) and `platformApi` (`greentic.callApi`, direct platform REST). Both are declared
empty here (`"ui": { "fetchHosts": [], "platformApi": [] }`), confirmed in
`describe.json`, and the page's own `bridge.js` never calls either — it only ever calls
`window.greentic.invokeTool`. The right to call the four tools this view uses
(`rag_list`, `rag_ingest`, `rag_delete`, `rag_search`) comes from `views[].tools` in
`contributions.views`, a separate grant from `ui`. `rag_upsert` and
`rag_collection_ensure` are deliberately excluded from that list: the page never needs
either, and `rag_ingest` already calls `ensure_collection` internally before every
write.

### Two claims worth checking before you copy this pattern

The following two claims were treated as unverified hearsay going in. Both were checked
directly against this repository's code, tests, and git history.

**1. No contributed view has ever been rendered by a real Greentic host — confirmed
true, and stronger than stated.** Searching the full tracked-and-untracked file listing
of this repository, its git history (`git log --all --diff-filter=A --name-only`), and
every mention of `iframe`/`mock`/`playwright`/`puppeteer`/`jsdom` anywhere in it turns up
nothing: there is no test harness, no HTML fixture, no JS test runner, and no CI
workflow directory (`.github/` does not exist) anywhere in this project's history. The
*only* automated check that touches the view assets at all is
`view_asset_tests::the_designer_and_admin_copies_of_the_view_are_identical` — a byte
comparison between two directories, which proves nothing about behavior — plus whatever
`gtdx lint`/`gtdx validate` check about `describe.json`'s `views[]` entries (paths
resolve, no remote assets, schema shape). `AGENTS.md`'s description of loading the page
in a sandboxed iframe against a page that speaks the bridge protocol and answers
`invokeTool` with canned results is written as an instruction for what exercising it
*would* require ("Exercising it means a host: …") — not as a description of a harness
that exists in this repository. So the claim as given is true, and the reality is
narrower than even "against a mock host" suggests: nothing here has *ever* rendered this
page, mocked or real. What is actually proven is: the manifest lints and validates, the
Rust build embeds the five files at compile time and asserts the two copies match, and
`gtdx dev --once` packs and installs the `.gtxpack` without error. Page behavior —
whether the file drop zone works, whether `pdf.js` actually extracts text correctly in a
real browser, whether the bridge's `event.source` check holds up against a real host —
is untested by anything that runs in this repository.

**2. Tenant isolation fails open, and `require_tenant_overlay` defaults to off —
confirmed true.** `src/config.rs`'s `Config::require_tenant_overlay` is a plain `bool`
with `#[serde(default)]`, which defaults to `false`; the test
`require_tenant_overlay_defaults_to_off` asserts exactly this on a config that omits the
field entirely. `ops::collection_of()` (see [The Rust guest](#the-rust-guest) above):
when no `_tenant_overlay` arrives on a call and the flag is off, the caller's
`collection` argument or the operator's configured default is used, with no way for the
guest to tell "this is a single-tenant install with nothing to stamp" apart from "this
host predates tenant stamping and never will stamp one." Turning the flag on closes
exactly that hole — `require_tenant_overlay_refuses_a_call_the_host_did_not_stamp`
confirms an unstamped call is then rejected with `PermissionDenied` rather than falling
back — but doing so unconditionally would also refuse every call on a single-tenant
install, which by definition never has an overlay to send. Both `src/config.rs`'s own
doc comment and `README.md`'s "Where this is still not airtight" section state this
plainly, and the code and tests back both statements up without qualification.

## Tenant isolation

Covered in depth in `README.md`'s "Collections and tenancy" section and this
document's [The Rust guest](#the-rust-guest) entry for `ops.rs`; not repeated here
beyond what a reader of this architecture document needs: the mechanism is
`ops::collection_of()`, the reserved `_tenant_overlay` argument key `input.rs` parses
into `TenantOverlay`, and the fail-open behavior verified above.

## Build, test and ship

**`Cargo.toml`** — `crate-type = ["cdylib"]` and nothing else (no `rlib`, no `[[bin]]`).
This forecloses two things: this crate can never be added as a normal Rust library
dependency of another crate (there is no `rlib` to link against), and it produces no
native executable. The only two ways to run any of its code are `cargo test` from
inside this crate, and the compiled `.wasm` component loaded through WIT by a host.
`package.metadata.component` points `cargo-component` at `wit/` and the three
`wit/deps/*` targets it depends on. Runtime dependencies: `wit-bindgen-rt`,
`serde`+derive, `serde_json`, `uuid` (with the `v5` feature, for `chunk_point_id`).
Dev-only: `greentic-extension-sdk-testing` (the `MockHttpClient`/`MockSecretsBackend`
mocks) and `serde_json` again for test bodies.

**`build.sh`** — three lines that matter: `cargo component build --release`, `mkdir -p
dist`, and an `ls -lh` of the produced `.wasm`. Its own comment is accurate: "Additional
packaging done by `gtdx publish`; this script just builds the wasm."

**`ci/local_check.sh`** — the full local gate, in order: `cargo fmt --all -- --check`,
`cargo clippy --all-targets -- -D warnings`, `cargo test`, `./build.sh`. See step 9 of
the walkthrough above for the known can't-stay-green-twice-in-a-row scaffold bug this
script has (regenerating an unformatted `bindings.rs` on its last step, which fails its
own first gate on the next run) — documented here as a fact about the tooling, not a
defect in this extension.

**`rust-toolchain.toml`** — pins `channel = "1.95.0"`, `targets = ["wasm32-wasip2"]`.
`rustup` activates this automatically inside the project directory; nothing else reads
it.

**`.gitignore`** — `/target`, `/dist`, `/src/bindings.rs`, `*.gtxpack`, `.DS_Store`.
Every one of these is a build artifact regenerated from tracked sources; none of them
should ever be committed.

## Docs and agent config

- **`README.md`** (751 lines) — the usage-facing document: quick start, configuration
  table, secrets, the full tool reference (schemas pulled straight from
  `tool_meta.rs`), a worked three-call example, the knowledge-base view walkthrough,
  four design decisions with the bugs each one fixed, requirements/limits, and its own
  copy of the architecture and testing sections. This document exists specifically not
  to repeat that content.
- **`AGENTS.md`** (265 lines) — the agent-facing map: file layout, the pure/host-boundary
  split, workflow commands, the three testing layers, the pre-publish self-check
  sequence, the contributed-view rules, tenant-isolation rules, secrets policy, and the
  generated-vs-hand-edited file list. The single most useful file in this repo for an
  agent working in it, and the primary source this document was cross-checked against.
- **`CLAUDE.md`** — one paragraph, entirely a pointer: "read `AGENTS.md` first," plus a
  two-sentence summary of what this extension is. Claude Code's actual entry point;
  `AGENTS.md` is where the real content lives.
- **`.claude/settings.json`** — pre-approves the build/test/gtdx Bash command prefixes
  (`cargo fmt`, `cargo clippy`, `cargo test`, `cargo build`, `cargo component build`,
  `gtdx`, `./build.sh`, `./ci/local_check.sh`) so running the gate does not prompt for
  permission on every step.
- **`.claude/commands/check.md`** — the `/check` slash command: runs the same six-step
  gate as `ci/local_check.sh` plus `gtdx validate`/`gtdx lint`, stopping at the first
  red step and reporting which one failed.
- **`i18n/en.json`** — three strings: `extension.name`, `extension.description`, and
  `view.knowledge.label` (the `title_key`/`title_fallback` source for both
  `contributions.views` entries).
- **`prompts/system.md`** — a hand-written system-prompt fragment, longer than and not
  generated from the runtime one. Its own header claims designer UIs load it when the
  extension is active; nothing in this repository's code or `describe.json` references
  its path, so that claim could not be verified against anything checked here — it is a
  convention the host side would have to implement, outside this repository's reach.
  Do not confuse it with `prompting::Guest::system_prompt_fragments()` in `src/lib.rs`,
  which is the text actually injected into an agentic worker's system prompt at call
  time; the two share their closing guidance ("call `rag_search` before answering from
  stored knowledge") but are different text serving different audiences — one is
  design-time UI copy, the other is the runtime instruction.
