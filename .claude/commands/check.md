---
description: Run the full pre-publish quality gate for this extension
---

Run the extension's quality gate in order. Stop at the first failure and report
exactly which step failed with its output — do not continue past a red step.

1. `cargo fmt --all -- --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test`
4. `cargo component build --release`
5. `gtdx validate`   — describe.json against the JSON Schema
6. `gtdx lint`       — describe.json cross-field invariants

If every step passes, state that the extension is ready.

To cut a release, additionally run `gtdx lint --publish` (rejects placeholder
`0000…` sha256) and then `gtdx publish`.
