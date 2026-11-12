# Publishing che-tauri

The current crates.io candidate is **0.2.0**. It requires **che-orm 0.4.0**;
feature `sync` also requires **che-sync 0.2.0**, which depends on
**che-sync-protocol 0.1.0**. The license is MIT. The independent
`examples/todo-sync` workspace and nested `che-back` repository are excluded.

## Current release status

The previous 0.1.2 candidate passed functional checks and packaging but was not security-cleared:
the optional credentials dependency chain still has five RustSec advisories after
compatible upgrades. See [the full verification report](release-verification-2026-11-06.md).
Do not interpret package/dry-run success as audit approval. No advisories have
been ignored and no publication has been performed.

Rust 1.90 is the declared and tested minimum for current Tauri dependencies;
Rust 1.88 is unsupported here even though the ORM/sync crates support it.

## Verify

```sh
cargo fmt --check
cargo +1.90.0 check --all-targets --all-features --locked
cargo check --all-targets --all-features
cargo test
cargo test --all-features
cargo clippy --all-targets --all-features -- -D warnings
cargo audit
cargo audit --file examples/todo-sync/Cargo.lock
cargo package --list
cargo publish --dry-run
```

Inspect the archive for its license, README, changelog, source and tests. Keep
credentials, local databases, build output and application workspaces out of it.
When dependencies have not been published yet, validate their archives together
in a temporary workspace; an ordinary registry dry run must be repeated once
they are indexed. A local path build does not prove registry availability.

Changes to the shared Todo example also require its checks from `AGENTS.md`.

## Publish

Publish and wait for dependency index entries in this order, skipping existing
versions: che-orm 0.4.0, then che-sync 0.2.0, then che-tauri 0.2.0.
Macros 0.3.2 and che-sync-protocol 0.1.0 remain unchanged and published.
che-rest 0.4.0 is required by the updated Todo example and optional HTTP batch integration,
but is not a direct library dependency. The che-tauri crates.io API returned
404 on 2026-11-06: this is its first publication. A later check during preparation confirmed che-sync 0.1.3
is now indexed, and the ordinary registry dry run passed. Once the unresolved
security findings are resolved/reviewed:

```sh
cargo publish --dry-run --locked
cargo publish --locked
```

The existing `v0.1.2` tag identifies an earlier candidate; do not move it.
The local integration uses a new `v0.2.0` tag; it does not identify a published
crate. No tag or archive alone proves a crates.io publication or docs.rs build.

## Candidate verification (2026-11-02)

Default and sync-feature tests, all-target/all-feature compilation and sync
Clippy with warnings denied passed. The shared Todo workspace passed its checks,
HTTP integration tests, Clippy, formatting and JavaScript syntax validation.
Tacit mobile built its generated TypeScript client and frontend, and passed its
native offline, attachment, comment, whiteboard and push integration tests.

The archive was packaged with the five sibling packages and passed an extracted
all-target sync build on current stable. Its MIT license and release documents
are included; neither che-back nor the application example is in the archive.
No crates.io publication was performed. Registry dry runs must be repeated once
the prepared dependencies are published and indexed.
