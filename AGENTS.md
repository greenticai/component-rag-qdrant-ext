# AGENTS.md

Guidance for AI coding agents (Claude Code, Codex, and similar) working in this
repository. This is a Greentic Designer **design** extension scaffolded by
`gtdx new` (SDK 1.2.9, contract `greentic:extension-design@0.3.0`).

> Claude Code reads `CLAUDE.md`, which points back here. Keep **this** file as the
> single source of truth for both tools and update it as the extension grows.

## What this is

- **id:** `greentic.rag-qdrant`  ·  **version:** `0.2.0`  ·  **kind:** `design`
- A WebAssembly component (target `wasm32-wasip2`) that the Greentic runtime loads
  as a signed `.gtxpack`.
- Exposes six tools to both flow nodes and the agentic worker, backed by a Qdrant
  vector collection: `rag_search`, `rag_upsert`, `rag_ingest`, `rag_delete`,
  `rag_collection_ensure`, `rag_list`. Text is embedded through a configurable
  OpenAI-shaped embeddings API; callers holding their own vectors can pass them
  directly. See `describe.json`'s `contributions.tools` for schemas, or
  `src/tool_meta.rs` for the Rust source of truth they're generated from.
- `describe.json` is the manifest and the single source of truth for metadata —
  the runtime and store read it.

## Layout

- `describe.json`        — extension manifest (id, version, capabilities, runtime)
- `src/lib.rs`            — WIT glue. The **only** module allowed to touch
  `crate::bindings`. Implements the WIT exports, maps `RagError` onto the WIT
  `extension-error`, and dispatches each tool name to `ops.rs`.
- `src/host.rs`           — the `HostCalls` trait (`fetch`, `secret`) that sits
  between the pure modules and the host. `lib.rs`'s `WitHost` is the only
  implementation backed by real WIT bindings; tests substitute
  `MockHttpClient` / `MockSecretsBackend` instead.
- `src/ops.rs`            — orchestration: the only module that sequences more
  than one host call per tool (embed → upsert; chunk → delete old → upsert
  new; …). Generic over `HostCalls`, so every sequence runs in `cargo test`.
- `src/tool_meta.rs`      — static metadata for the six tools (name,
  description, schemas, capabilities). A host test asserts it hasn't drifted
  from `describe.json`'s `contributions.tools`.
- `src/input.rs`          — JSON args in, validated typed values out, one
  parser per tool.
- `src/qdrant.rs`         — Qdrant REST request builders and response parsers.
- `src/embed.rs`          — OpenAI-shaped embeddings client (request/response).
- `src/chunk.rs`          — character-window text splitting for `rag_ingest`.
- `src/config.rs`         — operator configuration, parsed once in `lifecycle::init`.
- `src/error.rs`          — the extension's own error type; only `lib.rs` maps
  it onto the WIT `extension-error`, so no other module needs bindings.
- `wit/`                 — WIT contract, vendored by `gtdx new` (see `.gtdx-contract.lock`)
- `i18n/en.json`         — user-facing strings
- `build.sh`             — compile the wasm
- `ci/local_check.sh`    — fmt + clippy + test + build gate
- `.claude/`             — Claude Code config: pre-approved build perms + `/check` command

**Why the split:** reaching a WIT import from a host `cargo test` aborts the
process with SIGABRT — non-unwinding and uncatchable, nothing to `Result` on.
So every host call is injected behind `HostCalls` (`host.rs`) rather than
called directly, and substituted with `MockHttpClient` / `MockSecretsBackend`
(from `greentic-extension-sdk-testing`) in tests. That's what makes this
extension's logic runnable in milliseconds on the host instead of only inside
a WASM runtime. If you copy this layout: `bindings::` calls belong in
`lib.rs` only — one in a pure module will pass `cargo check` and then SIGABRT
the moment a test reaches it.

## Workflow

```
gtdx dev        # watch: rebuild + reinstall to the local registry on save
gtdx publish    # build, pack to dist/greentic.rag-qdrant-<version>.gtxpack, install locally
./build.sh      # just compile the wasm (cargo component build --release)
./ci/local_check.sh   # fmt + clippy -D warnings + test + build
```

Run `./ci/local_check.sh` before every commit and before publishing. The bar is
**zero clippy warnings** (`cargo clippy --all-targets -- -D warnings`) and green
tests. Do not commit if it fails.

**Known upstream issue:** this script cannot stay green across two consecutive
runs in *any* `gtdx new`-scaffolded extension, this one included. Its first
gate is `cargo fmt --all -- --check`; its last step, `./build.sh`, runs
`cargo component build --release`, which regenerates the gitignored
`src/bindings.rs` — unformatted. Run the script again right after and the fmt
gate fails on output the script itself just produced. This is a scaffold bug
being fixed upstream in the SDK, not a defect in this extension — if you hit
it, it's expected: just `cargo fmt` before your next commit.

## Testing

Three layers, fastest first. Most of your testing belongs in the first one.

| Layer | Command | What it proves |
|---|---|---|
| Unit, on the host | `cargo test` | Your logic is right. Milliseconds, no WASM, no designer. |
| Full gate | `./ci/local_check.sh` | fmt + clippy + tests + the wasm actually builds. |
| Integration | `gtdx dev --once` | It packs and installs. Not that it behaves — that is the layer above. |

The guest exports are plain Rust functions, so a host test calls them directly:

```rust
let out = <Component as tools::Guest>::invoke_tool(tool_meta::SEARCH_TOOL.to_string(), args)?;
```

`src/lib.rs` ships tests doing exactly this — asserting the WIT-facing shape
(tool count, metadata, error mapping) rather than Qdrant behavior, which lives
in `ops.rs`'s own host-call tests. Extend them; do not delete them and leave
`cargo test` with nothing to run.

**One prerequisite:** `cargo test` needs `src/bindings.rs`, which is generated
rather than committed. Build once (`gtdx dev --once` or `cargo component build`)
before the first `cargo test` in a fresh clone, or it fails with
`cannot find export in bindings`.

Testing code that calls host functions — http, secrets, state, logging,
translation? Add the SDK's mocks as a dev-dependency:

```toml
[dev-dependencies]
greentic-extension-sdk-testing = "1.2.9"
```

They are ordinary in-memory objects (`MockHttpClient`, `MockSecretsBackend`,
`MockLogger`, …), so they only help if your code takes the host dependency as a
parameter rather than calling the binding directly. Structure it that way and
the logic stays testable on the host.

## Self-check before publishing

Beyond the Rust gate above, validate the manifest with gtdx — these catch a broken
`describe.json` that compiles fine but the runtime/store will reject:

```
gtdx doctor     # environment: cargo, cargo-component, wasm32-wasip2 target
gtdx validate   # describe.json against the JSON Schema
gtdx lint       # describe.json cross-field invariants (id pattern, schema host, …)
gtdx lint --publish   # also rejects placeholder 0000… sha256 (E_SHA256_ZERO)
gtdx verify     # verify the signature once published/signed
```

Run `gtdx validate` and `gtdx lint` after any edit to `describe.json`, and
`gtdx lint --publish` right before `gtdx publish`.

## Already past scaffold

`gtdx new` seeds an id namespace, sample metadata, and one placeholder
tool/node meant to be replaced with the real surface. That's done here — there
is no leftover scaffold value to hunt down:

- **id** is `greentic.rag-qdrant`, not the `com.example.*` reverse-DNS sample.
  It and its WIT-package form (`greentic:rag-qdrant`) stay in sync everywhere
  they appear — `describe.json` (`metadata.id`, `runtime.world`,
  `runtime.components` key), `Cargo.toml` (`package.metadata.component.package`),
  `wit/world.wit` (the `package` line). If you ever rename the id, update all
  four together.
- **metadata** (`name`, `summary`, `description`, `author`) in `describe.json`
  describes this extension, not the sample.
- **the placeholder export is gone** — `src/lib.rs`'s `dispatch()` routes to
  the six real tools (see Layout above); there is no sample echo tool left.
- **i18n** (`i18n/en.json`) holds this extension's own strings.

If you're using this repo as a reference for a new extension, the pattern to
copy is the pure/host-boundary module split in Layout above, not this list.

## Secrets

Never hardcode credentials or API keys in `src/lib.rs`. If the extension needs a
secret, declare it under `secret_requirements` in `describe.json`; the runtime
resolves and injects it at execution time.

## Do NOT hand-edit these — generated or managed

- **`sha256` fields in `describe.json`** — the `0000…0000` values are placeholders.
  `gtdx publish` computes the real content hashes. Never fill them in by hand.
- **`.gtdx-contract.lock`** — generated by `gtdx new`; pins the vendored WIT
  contract hashes. Don't edit.
- **`wit/deps/`** — vendored WIT contract. Treat as read-only; it is locked.
- **`src/bindings.rs`** — generated bindings (gitignored). Never commit or edit.
- **`target/`, `dist/`, `*.gtxpack`** — build output (gitignored).

## Conventions

- Rust edition 2024; toolchain pinned in `rust-toolchain.toml` — don't float it.
- Build target is `wasm32-wasip2` only.
- On every release, bump `[package].version` (`Cargo.toml`), `metadata.version`
  (`describe.json`), and `runtime.components.*.gtpack.component_version`
  (`describe.json`) together. The third one is hand-maintained — unlike the
  `sha256` fields next to it, `gtdx publish` never writes it, so it drifts
  silently if you forget it.
- `describe.json` must stay valid v2 shape: `apiVersion: greentic.ai/v2`, a `compat`
  block, and a `runtime.components` map.
- JSON for `describe.json`, YAML for human-authored configs, CBOR for binary metadata.
