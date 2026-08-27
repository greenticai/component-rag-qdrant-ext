# component-rag-qdrant-ext

A Greentic Designer **design** extension exposing six RAG tools — over both
flow nodes and the agentic worker — backed by a Qdrant vector collection:
`rag_search`, `rag_upsert`, `rag_ingest`, `rag_delete`, `rag_collection_ensure`,
`rag_list`. Text is embedded through a configurable OpenAI-shaped embeddings
API; callers that already hold a vector can pass it directly instead.

- id: `greentic.rag-qdrant`
- version: `0.3.0`
- contract: `greentic:extension-design@0.3.0`
- published: `greentic.rag-qdrant@0.3.0` on the Greentic store

**If you are here to copy this repo as a starting point for your own
extension**, the RAG/Qdrant part is the least important thing in it. What is
worth copying is the *shape*: a WIT boundary reduced to one file, every other
module pure and runnable in `cargo test` without a WASM runtime, and the
handful of ordering bugs that shape was built to make impossible. Read
[Architecture](#architecture) and [Design decisions worth stealing](#design-decisions-worth-stealing)
before you write your first `bindings::` call.

## Quick start

### Prerequisites

- Rust via `rustup` — the toolchain is pinned in `rust-toolchain.toml`
  (`1.95.0`, target `wasm32-wasip2`) and activates automatically in this
  directory.
- [`cargo-component`](https://github.com/bytecodealliance/cargo-component).
- The `gtdx` CLI. Run `gtdx doctor` to confirm cargo, cargo-component, and the
  `wasm32-wasip2` target are all in place.

### Build and install locally

```
gtdx dev --once
```

Builds, packs to `dist/greentic.rag-qdrant-<version>.gtxpack`, and installs it
into your local registry (`~/.greentic`). This is also what generates
`src/bindings.rs` — a gitignored file that `cargo test` needs, so run this (or
`cargo component build`) once on a fresh clone before your first `cargo test`,
or it fails with `cannot find export in bindings`.

### Configure

A host loading this extension calls `lifecycle::init` with a JSON body shaped
like the [Configuration](#configuration) table below, e.g.:

```json
{
  "qdrant_url": "https://xyz.qdrant.io:6333",
  "collection": "kb",
  "embedding": {
    "base_url": "https://api.openai.com/v1",
    "model": "text-embedding-3-small",
    "dimensions": 1536
  }
}
```

### Secrets

This repo has no CLI for setting secret *values* — the host that loads the
extension (a Designer instance or the agentic worker) resolves
`secret://rag-qdrant/qdrant_api_key` and `secret://rag-qdrant/embedding_api_key`
at call time and injects them; see [Secrets](#secrets). Nothing in this
repository ever sees the raw key.

### See a tool call actually run

There is no live Qdrant cluster in this repo, so the fastest way to watch a
tool execute the real code path — argument parsing, config lookup, the host
calls, response assembly — is the test suite. It runs the same `ops::*`
functions `lib.rs`'s `dispatch()` calls, against a mocked Qdrant and a mocked
embeddings API instead of the network:

```
cargo test ingest_deletes_the_document_before_upserting_its_chunks -- --nocapture
```

or run everything with `cargo test` (115 tests, milliseconds, no WASM runtime
involved — see [Testing](#testing)). To call a tool for real, install the
built `.gtxpack` (above) into a Designer or agentic worker instance and give
it the config and secrets above — that step happens outside this repo.

Before publishing anything, validate the manifest:

```
gtdx validate && gtdx lint
```

## Configuration

Passed to `lifecycle::init` as JSON:

| Field | Required | Default | Notes |
|---|---|---|---|
| `qdrant_url` | yes | — | e.g. `https://xyz.qdrant.io:6333`. Trailing slash is stripped. |
| `collection` | yes | — | Fallback collection, used when the host sends no per-tenant one. See [Collections and tenancy](#collections-and-tenancy). |
| `embedding.base_url` | yes | — | `/embeddings` is appended. Any OpenAI-shaped API. |
| `embedding.model` | yes | — | e.g. `text-embedding-3-small`. |
| `embedding.dimensions` | yes | — | Must match the collection's vector width. |
| `chunk.max_chars` | no | `1200` | Characters, not bytes. |
| `chunk.overlap_chars` | no | `150` | Must be less than `max_chars`. |
| `require_tenant_overlay` | no | `false` | Refuse any call the host did not stamp a tenant collection onto, instead of falling back to `collection`. **Turn this on for a multi-tenant install** — see [Collections and tenancy](#collections-and-tenancy). |

## Secrets

`rag-qdrant/qdrant_api_key` and `rag-qdrant/embedding_api_key`.

## Tool reference

Every schema below is the actual `input_schema_json` / `output_schema_json`
from `src/tool_meta.rs` (also what `describe.json`'s `contributions.tools` is
generated from — a test asserts the two never drift apart). Response shapes
are what `src/ops.rs` actually constructs, which is occasionally a superfield
of the declared output schema (the schema states what's *guaranteed*, not
everything the response carries).

Three rules apply across all six tools and are worth internalizing before
reading the individual schemas:

- `rag_search` and `rag_upsert` take *either* a text field *or* a `vector`,
  never both and never neither. The schema encodes it as a two-branch `oneOf`
  (`required: ["query"], not: {required: ["vector"]}` and the mirror image),
  and `src/input.rs` enforces the same rule again at parse time — so a model
  that only reads `required`/`properties` and ignores `oneOf` still gets a
  clear `InvalidInput` rather than silently picking one.
- `rag_delete` takes *either* `ids` *or* `doc_id`, same two-branch `oneOf`.
  There is deliberately no "delete everything" path — passing neither is
  rejected, not treated as "all".
- `rag_upsert`'s `id` must be an unsigned integer or a UUID (checked with
  `id.parse::<u64>()` or `Uuid::parse_str`); anything else is rejected before
  any network call, because Qdrant would otherwise reject it with an opaque
  400 after an embeddings call had already been spent.

### `rag_search`

Semantic search over the Qdrant knowledge base. Read-only, no confirmation
required by the agentic worker.

| Field | Type | Required | Notes |
|---|---|---|---|
| `query` | string | one of `query`/`vector` | Text to search for. |
| `vector` | number[] | one of `query`/`vector` | A pre-computed embedding. |
| `top_k` | integer ≥ 1 | no | How many results (default 5). |
| `filter` | object | no | A Qdrant filter object, passed through verbatim. |
| `collection` | string | no | Override the configured default collection. |

```json
// request
{ "query": "what is a design extension", "top_k": 2 }
```

```json
// response
{
  "hits": [
    { "id": "ceca5e3a-9d3e-5f77-91ae-f4254a3a736d", "score": 0.83, "payload": { "doc_id": "greentic-overview", "chunk_index": 0, "text": "...", "source": "docs/overview.md" } }
  ]
}
```

### `rag_upsert`

Store or replace a single point by id.

| Field | Type | Required | Notes |
|---|---|---|---|
| `id` | string | yes | Unsigned integer or UUID. |
| `text` | string | one of `text`/`vector` | Embedded and stored. |
| `vector` | number[] | one of `text`/`vector` | A pre-computed embedding. |
| `payload` | object | no | Arbitrary metadata stored alongside the point. |
| `collection` | string | no | Override the configured default collection. |

```json
// request
{ "id": "42", "text": "the greenhouse opens at 08:00", "payload": { "lang": "en" } }
```

```json
// response
{ "ok": true, "id": "42" }
```

### `rag_ingest`

Chunk, embed and store a whole document under a `doc_id`. Re-ingesting the
same `doc_id` replaces its chunks — see
[Design decisions worth stealing](#design-decisions-worth-stealing). Writes,
so the agentic worker asks for confirmation.

| Field | Type | Required | Notes |
|---|---|---|---|
| `doc_id` | string | yes | Stable document identifier. |
| `text` | string | yes | Full document text; chunked, embedded and stored. |
| `metadata` | object | no | Copied onto every chunk of this document. |
| `collection` | string | no | Override the configured default collection. |

```json
// request
{
  "doc_id": "greentic-overview",
  "text": "Greentic Designer extensions are signed WASM components loaded by the runtime.",
  "metadata": { "source": "docs/overview.md" }
}
```

```json
// response
{ "ok": true, "doc_id": "greentic-overview", "chunks": 1 }
```

### `rag_delete`

Delete points, either by explicit ids or by deleting every chunk of one
`doc_id`. Not recoverable.

| Field | Type | Required | Notes |
|---|---|---|---|
| `ids` | string[] | one of `ids`/`doc_id` | Point ids to delete. |
| `doc_id` | string | one of `ids`/`doc_id` | Delete every chunk of this document. |
| `collection` | string | no | Override the configured default collection. |

```json
// request
{ "doc_id": "greentic-overview" }
```

```json
// response
{ "ok": true }
```

### `rag_collection_ensure`

Create the collection if it does not exist, fixing its vector width and
distance metric. Safe to call repeatedly — `rag_ingest` and `rag_upsert`
already call it internally before every write.

| Field | Type | Required | Notes |
|---|---|---|---|
| `collection` | string | no | Defaults to the configured collection. |
| `dimensions` | integer ≥ 1 | no | Defaults to the configured `embedding.dimensions`. |
| `distance` | `"Cosine"` \| `"Dot"` \| `"Euclid"` | no | Default `Cosine`. |

```json
// request
{}
```

```json
// response
{ "ok": true, "collection": "kb", "dimensions": 1536, "distance": "Cosine" }
```

### `rag_list`

Enumerate stored documents, grouped by `doc_id`, with pagination.

| Field | Type | Required | Notes |
|---|---|---|---|
| `limit` | integer ≥ 1 | no | Max chunks scanned per page (default 50) — a page size over chunks, not documents. |
| `offset` | any | no | Opaque cursor from a previous response's `next_page_offset`. |
| `filter` | object | no | A Qdrant filter object, passed through verbatim. |
| `collection` | string | no | Override the configured default collection. |

```json
// request
{}
```

```json
// response
{
  "documents": [
    { "doc_id": "greentic-overview", "chunk_count": 1, "metadata": { "source": "docs/overview.md" } }
  ]
}
```

`next_page_offset` is present only when Qdrant's scroll has another page —
pass it back verbatim as `offset` on the next call. `chunk_count` counts only
the chunks that landed in the *current* page: a document whose chunks straddle
a page boundary shows a partial count on each page it appears on, rather than
this tool looping internally and risking a silent truncation on a large
collection.

## Worked example

Ingest a short document, search it, then list it — in sequence, against the
same collection.

**1. `rag_ingest`** — chunk, embed, and store the document. `metadata` is
copied onto every chunk; here the text is short enough for a single chunk.

```json
{ "doc_id": "greentic-overview", "text": "Greentic Designer extensions are signed WASM components loaded by the runtime.", "metadata": { "source": "docs/overview.md" } }
```
```json
{ "ok": true, "doc_id": "greentic-overview", "chunks": 1 }
```

**2. `rag_search`** — find it by meaning. The chunk's point id is the
deterministic UUIDv5 of `"greentic-overview:0"`; its payload carries the
`doc_id`/`chunk_index`/`text` `rag_ingest` wrote plus the caller's own
`source` metadata.

```json
{ "query": "what is a design extension", "top_k": 1 }
```
```json
{
  "hits": [
    {
      "id": "ceca5e3a-9d3e-5f77-91ae-f4254a3a736d",
      "score": 0.83,
      "payload": {
        "doc_id": "greentic-overview",
        "chunk_index": 0,
        "text": "Greentic Designer extensions are signed WASM components loaded by the runtime.",
        "source": "docs/overview.md"
      }
    }
  ]
}
```

**3. `rag_list`** — confirm what's stored. The response strips the chunk
bookkeeping fields (`doc_id`, `chunk_index`, `text`) back out of the payload,
leaving only the caller's original `metadata`.

```json
{}
```
```json
{
  "documents": [
    { "doc_id": "greentic-overview", "chunk_count": 1, "metadata": { "source": "docs/overview.md" } }
  ]
}
```

## Knowledge base view

It exists so someone who will never call a tool by hand can still curate the
knowledge base: list what is stored, upload a document, delete one, and search.
It ships on **both** host surfaces.

`describe.json` contributes it under `contributions.views[]` as two entries:

```json
[
  {
    "id": "knowledge",
    "surface": "designer",
    "entry": "index.html",
    "placement": { "slot": "designer.sidebar", "order": 20 },
    "min_visibility": "member",
    "tools": ["rag_list", "rag_ingest", "rag_delete", "rag_search"]
  },
  {
    "id": "knowledge-admin",
    "surface": "admin",
    "entry": "index.html",
    "placement": { "slot": "admin.sidebar", "order": 20 },
    "min_visibility": "tenant_admin",
    "tools": ["rag_list", "rag_ingest", "rag_delete", "rag_search"]
  }
]
```

(`title_key` and `title_fallback` elided above; both entries use
`view.knowledge.label` / "Knowledge base".)

#### Why two entries, and two copies of the page

`Surface` holds a single value, and view ids must be unique across the whole
array because the host namespaces them into a route (`<extension id>/<view
id>`), so one entry can never cover both hosts.

The awkward part is the assets. `gtdx lint` resolves `entry` at
`assets/views/<view id>/<entry>`, and `entry` may not contain `..`, so **each
id needs its own directory** — the two entries cannot point into one shared
bundle.

A symlinked second directory looks like it solves this and does not: lint
follows the symlink and passes, while the packer copies only real files, so the
`.gtxpack` ships **nothing** under the symlinked id. That combination — lint
clean, pack broken — is worse than the duplication, so the page is duplicated
for real into `assets/views/knowledge-admin/`.

The two copies are byte-identical and must stay that way; the page is
surface-agnostic, and anything that ever needs to differ should branch on
`surface` from the host's `init` message rather than fork the file. A test,
`view_asset_tests::the_designer_and_admin_copies_of_the_view_are_identical`,
fails the build if they drift. The cost is that the pack carries the bundle
twice, about 115 KB.

### What the page may reach

Nothing directly. It runs in an iframe with `sandbox="allow-scripts"` and no
`allow-same-origin`, so it has an opaque origin: no host cookies, no
`localStorage`, no parent DOM, and its own `fetch()` would send `Origin: null`.
Every byte it displays arrives through `window.greentic.invokeTool`, which the
host executes with the *viewer's* permissions — so the page can never see more
than the person looking at it could see by hand, and no credential ever crosses
into the browser.

`runtime.permissions.ui` is therefore declared empty:

```json
"ui": { "fetchHosts": [], "platformApi": [] }
```

That is not an oversight. `ui` grants only two things — a server-side proxied
`greentic.fetch` and platform REST via `greentic.callApi` — and this page uses
neither. The right to call the four tools comes from `views[].tools`, not from
`ui`. `rag_upsert` and `rag_collection_ensure` are deliberately absent: the page
never needs them, and `rag_ingest` ensures the collection itself.

Because the page is fed attacker-influenced strings — anyone who can ingest
chooses a `doc_id`, the document text and the metadata — everything
tool-derived is written with `textContent` and never `innerHTML`. Script
injected into this frame would inherit the bridge, and with it the right to
call every tool the view declares.

### Reading files in the browser

The host exposes no filesystem, and `rag_ingest` takes text. So the page does
the conversion itself, before anything is sent:

| Format | Handling |
|---|---|
| `.txt`, `.text`, `.csv`, `.tsv` | Decoded as UTF-8 strictly; falls back to Windows-1252 with a visible warning. Bytes containing NUL are rejected as binary. |
| `.md`, `.markdown`, `.mdown`, `.mkd` | Same as plain text; the Markdown is ingested as-is. |
| `.pdf` | Text extracted in-page by `pdf.js` — see below. |
| anything else | Rejected by name, with a message naming the formats that do work. |

`pdf.js` is a dependency-free PDF text extractor: it
inflates `/FlateDecode` streams with the browser's own `DecompressionStream`,
walks the page tree (including `/ObjStm` object streams and PNG predictors),
and maps glyphs to Unicode via `/ToUnicode` CMaps, then `/Differences`
encodings, then the standard Latin encodings. **It rejects rather than
guesses**, because a mangled decode ingested into a knowledge base is close to
undetectable afterwards — it looks fine in a listing and only ever shows up as
bad search results. Encrypted PDFs, PDFs with no text layer (scans), and any
extraction whose decoding confidence falls below the threshold are refused with
a plain-English reason instead of being ingested. Identity-H CID fonts with no
`/ToUnicode` map are not recoverable and fail this check by design.

Whatever the format, the extracted text is shown in a preview **before** the
user commits, so an imperfect extraction is caught by a human rather than
silently stored.

### Things worth knowing before you change it

- **No remote assets, ever.** `gtdx lint` fails a remote `<script src>`,
  `<img src>` or `<link href>` in the entry HTML with `E_VIEW_REMOTE_ASSET`:
  the manifest sha256 would otherwise cover a file that pulls unverified code
  at runtime. That rules out a CDN and therefore any framework — the page is
  plain JS on purpose. Note the rule only scans the entry HTML, so a remote
  URL assembled at runtime inside `app.js` would slip past it; don't.
- **`bridge.js` is copied byte-for-byte from the SDK scaffold.** Its
  `postMessage` handler checks `event.source === window.parent` and
  deliberately *not* `event.origin`, because an opaque origin arrives as the
  literal string `"null"` and a forged message would look identical by that
  measure. Do not "fix" it.
- **No `window.confirm` / `window.alert`.** A native modal blocks the frame's
  event loop, and with it the `message` handler every bridge reply arrives on;
  each in-flight call would then time out. Delete confirmation is in-page for
  this reason.
- **Every bridge call times out after 10 seconds**, `rag_ingest` included.
  `rag_ingest` embeds all of a document's chunks in one request before it
  writes, so a long document can exceed that. The page warns above ~150k
  characters and, on a timeout, says the ingest may have succeeded anyway and
  to refresh — which is true, and safe to retry, because re-ingesting a
  `doc_id` replaces rather than duplicates.
- **Large files must not freeze the tab.** File reading and PDF extraction
  yield to the event loop and report progress; the preview is capped.

### Who may see it

The two entries carry different floors, because their audiences differ.

**Designer — `member`.** It was `tenant_admin` while the collection was
process-wide and a caller could name any collection it liked; with the host now
stamping the tenant's collection and a per-call override refused, a member
cannot reach another tenant's data through this page. `min_visibility` is a
*floor*, not a grant: platform admins decide which tenants get the extension at
all, and tenant admins decide placement and which of their teams actually see
the view. A `member` floor lets a tenant admin delegate knowledge curation to a
content team — which is the whole point of a page aimed at non-technical
curators. A `tenant_admin` floor would forbid that permanently, in a signed
artifact.

**Admin console — `tenant_admin`.** The Admin console is an operator surface;
practically everyone who can open it is already a tenant admin or above, so a
`member` floor there would claim an audience that does not exist and understate
who the page is for. Declaring `tenant_admin` says what is actually true.

Note this is about *isolation*, which is now handled below the UI. The page can
delete any document in its tenant, and deciding who inside a tenant may do that
is tenant and team configuration — not something either floor settles.

### Collections and tenancy

**The page never sends a `collection` argument**, and under a multi-tenant host
it would be refused if it did.

The host stamps a reserved argument, `_tenant_overlay`, onto every tool call on
both surfaces. It carries this extension's *effective* configuration for the
calling tenant — the operator baseline deep-merged with that tenant's override,
resolved from the admin's `extension_config` tables. Crucially, both hosts
**strip `_tenant_overlay` from the caller's own arguments unconditionally** and
re-insert their own, including when a tenant has no override configured. That
unconditional strip is what makes it trustworthy: without it, a page could
smuggle `_tenant_overlay: {"collection": "someone-elses"}` during the window
when no override happened to be set.

`collection_of()` in `src/ops.rs` therefore resolves, highest first:

1. `_tenant_overlay.collection` — host-set, unforgeable, the isolation boundary.
2. the caller's `collection` argument — only when no overlay pins one.
3. `collection` from the operator config.

**A caller `collection` is refused whenever the overlay pins one — even when it
matches.** Silently ignoring a disagreement would let a flow author believe they
were reading a collection they were not. Refusing only a disagreement would
leave the matching case as a quiet invitation to hard-code a tenant's collection
name into a flow: it reviews cleanly, works until the tenant is reconfigured or
the flow is copied to another tenant, then fails somewhere far away. One total
rule is easier to document, test and act on than a rule with an exception in it.

Step 2 is what keeps single-tenant installs and local development working. With
no overlay there is nothing to undermine, and `collection` behaves exactly as it
always has.

The resolution happens **before** any host call — before chunking, before
embedding. A refusal must not first bill the operator for embedding a document
it is about to reject, and an authorisation check belongs ahead of the side
effects rather than between them.

#### Where this is still not airtight

- **It fails open on a host that does not stamp.** With no overlay the guest
  cannot tell "single-tenant install" from "host too old to stamp one", and the
  second silently serves every tenant the same collection. Set
  **`require_tenant_overlay: true`** on any multi-tenant install: unstamped
  calls are then refused rather than quietly falling back. It is off by default
  only because turning it on would break every single-tenant install.
- **Only `collection` is applied from the overlay.** If an operator isolates
  tenants by *cluster* instead, a differing `_tenant_overlay.qdrant_url` is
  **refused** with a clear error rather than ignored — every request builder
  here reads `qdrant_url` off the process config, so honouring the collection
  while silently using the baseline cluster would be exactly the cross-tenant
  read to avoid. An overlay that merely repeats the configured URL is accepted.
- **`embedding` and `chunk` in the overlay are ignored.** A tenant that
  overrides `embedding.dimensions` would get vectors of the process-wide width;
  Qdrant rejects a wrong-width write, so this fails loudly rather than
  corrupting an index — but it is not *supported*.
- **Isolation is not authorization.** Per-tenant collections stop tenant A
  reading tenant B. They say nothing about which people *inside* a tenant may
  delete its documents.

## Architecture

For the file-by-file inside view — what every module, manifest field and view
asset does, a from-scratch build walkthrough, and what is and is not proven
about the contributed view — see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

The one rule that shapes every other file in `src/`: **every module is pure
and host-testable except `src/lib.rs`.**

That split is forced, not a style preference. This extension compiles to
`wasm32-wasip2` and its host calls (`http::fetch`, `secrets::get`) are WIT
imports. Reach one of those imports from a plain host `cargo test` — no WASM
runtime underneath — and the process aborts with `SIGABRT`: non-unwinding,
uncatchable, nothing to `Result` on, and it takes the entire test binary down
with it, not just the one test. There is no way to `catch_unwind` around it.

So every host call is injected behind the `HostCalls` trait (`src/host.rs`):
`fetch` and `secret`. `src/lib.rs`'s `WitHost` is the only implementation
backed by real WIT bindings, and it is also the only place in the crate
allowed to import `bindings::`. Everywhere else — `ops.rs`, `qdrant.rs`,
`embed.rs`, `chunk.rs`, `config.rs`, `input.rs`, `error.rs` — takes `&impl
HostCalls` generically, and tests substitute the SDK's `MockHttpClient` /
`MockSecretsBackend` (from `greentic-extension-sdk-testing`) instead of a real
transport. That's what makes 115 tests run in milliseconds on the host instead
of requiring a WASM runtime or a live Qdrant cluster.

`src/ops.rs` is the only module that sequences more than one host call for a
single tool (embed → ensure → delete → upsert for `rag_ingest`; embed →
query for `rag_search`; …). That is deliberate: since ordering only exists
where more than one call happens, ordering bugs — the delete-before-upsert
and embed-before-delete rules below — can only live in this one file. Every
other module does at most one thing to one host call's worth of data.

If you copy this layout for a new extension: `bindings::` calls belong in
`lib.rs` only. A `bindings::` call in a pure module passes `cargo check`
without complaint and then `SIGABRT`s the instant a test reaches it — a
failure mode with no useful backtrace, so it's worth getting this right
before writing your first tool.

## Testing

```
cargo test                    # 115 tests, ~milliseconds, no WASM runtime
./ci/local_check.sh           # fmt + clippy -D warnings + test + build
gtdx validate && gtdx lint    # describe.json against schema + cross-field invariants
```

There are 115 unit tests and zero integration tests against a real Qdrant.
That is the point of the pure/host-boundary split above: every tool's logic —
argument validation, request construction, response parsing, and the
ordering across multiple host calls — runs against `MockHttpClient` /
`MockSecretsBackend`, which are in-memory stand-ins the SDK provides for
exactly this. `MockHttpClient` matches requests on method + URL, returns
canned responses, records every call for assertions, and can assert a run
never left the declared network allowlist. `MockSecretsBackend` is an
in-memory secret store. Nothing here talks to a network, a filesystem, or a
WASM runtime, which is what keeps the whole suite in the millisecond range.

Here is a test driving `rag_ingest` end to end through the mocks — asserting
the exact ordering guarantee described below (quoted from `src/ops.rs`):

```rust
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
```

Three layers exist beyond `cargo test`, fastest first — see `AGENTS.md` for
the full breakdown:

| Layer | Command | What it proves |
|---|---|---|
| Unit, on the host | `cargo test` | Your logic is right. |
| Full gate | `./ci/local_check.sh` | fmt + clippy + tests + the wasm actually builds. |
| Integration | `gtdx dev --once` | It packs and installs. Not that it behaves — that's the layer above. |

## Design decisions worth stealing

Each of these fixed a real bug found in review. The reasoning is worth
copying along with the code.

- **Chunk point ids are deterministic.** `qdrant::chunk_point_id` computes a
  UUIDv5 over `"{doc_id}:{chunk_index}"` against a fixed namespace constant.
  Re-ingesting a document produces the *same* point ids for the same chunks,
  so Qdrant's upsert overwrites them in place instead of accumulating
  duplicates every time a document changes.
- **`rag_ingest` deletes a document's existing chunks before upserting the
  new ones**, not after. If a document shrinks from 5 chunks to 3 on
  re-ingest and the delete ran last (or not at all), the old chunks 3 and 4
  would survive with stale content and keep matching searches forever —
  orphans with no way for a caller to know they're stale.
- **Embedding happens before the delete.** The order inside `ingest` is
  chunk → embed → ensure collection → delete old chunks → upsert new ones.
  If the embeddings call fails, execution stops before the delete runs, so a
  failed re-ingest leaves the previous, working version of the document
  intact instead of leaving the knowledge base empty.
- **`rag_list` picks the surviving metadata by lowest `chunk_index`**, not by
  whichever point Qdrant's scroll happened to return first. Scroll order
  isn't guaranteed stable across calls, so picking "whatever came first"
  would let the same `rag_list` call return different metadata for the same
  document on different runs. Ranking by `chunk_index` (points without one
  ranking last, ties broken by point id) makes the winner deterministic
  instead.

## Requirements and limits

- **Qdrant 1.10 or newer.** Search uses the Query API (`/points/query`); the
  legacy `/points/search` path is not used.
- **`runtime.permissions.network` in `describe.json` grants `https://*.qdrant.io/*`.**
  Qdrant **Cloud** works out of the box — that wildcard covers every tenant's
  cluster host, no editing required. **Self-hosted** Qdrant is not covered:
  its host must be added to the allowlist by hand. Likewise, if
  `embedding.base_url` is configured to anything other than
  `https://api.openai.com`, that host must be added to the same allowlist too,
  or the embeddings call is rejected before it ever reaches the network.
  (The wildcard is verified against the SDK's host mock, which matches on
  host only; if a runtime instead matches on the full URL including port, a
  `:6333` variant of the pattern may be needed.)
- **Ingestion takes text, not files.** The host exposes no filesystem, so
  `rag_ingest` is given text, never a file. The contributed
  [knowledge base view](#knowledge-base-view) works around this for the formats
  a browser can read — it extracts plain text, Markdown and PDF in the page and
  sends the text — but a caller driving `rag_ingest` from a flow must convert
  first, and DOCX is not supported anywhere.
- **Designer 1.2.0 or newer** (`compat.min_designer_version`). The published
  designer at 0.6.0 cannot run this extension.

## Develop

```
gtdx dev           # watch, rebuild, and reinstall to local registry on save
```

## Publish

```
gtdx publish       # produce dist/greentic.rag-qdrant-<version>.gtxpack + install to local registry
```

## Layout

- `describe.json` — extension manifest
- `src/lib.rs`    — WIT glue: the only module that touches `crate::bindings`;
  dispatches each tool to `ops.rs`
- `src/host.rs`   — the `HostCalls` trait standing in for WIT host calls in tests
- `src/ops.rs`    — orchestration: the only module that sequences more than one
  host call per tool
- `src/tool_meta.rs` — static tool metadata (schemas, capabilities, agentic-worker hints); also generates `describe.json`'s `contributions.tools` block
- `src/input.rs`, `src/qdrant.rs`, `src/embed.rs`, `src/chunk.rs`, `src/config.rs`, `src/error.rs`
  — pure modules: argument parsing, Qdrant request/response handling, the
  embeddings client, text chunking, operator config, and the error type
- `assets/views/knowledge/` — the contributed view: `index.html`, `style.css`,
  `app.js`, the dependency-free `pdf.js` text extractor, and `bridge.js`
  (copied verbatim from the SDK scaffold)
- `assets/views/knowledge-admin/` — a byte-identical copy of the above, because
  each view id needs its own directory. Kept in step by a test; see
  [Why two entries, and two copies of the page](#why-two-entries-and-two-copies-of-the-page)
- `wit/`          — WIT contract (vendored by `gtdx new`; see `.gtdx-contract.lock`)
- `i18n/en.json`  — user-facing strings
- `AGENTS.md`     — guidance for AI coding agents (Claude Code, Codex, …); see
  it for the full pure/host-boundary module split, the workflow commands, and
  what's already past scaffold
- `CLAUDE.md`     — Claude Code entry point (points to `AGENTS.md`)
- `.claude/`      — Claude Code config: pre-approved build perms + `/check` command
