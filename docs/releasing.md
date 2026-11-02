# Publishing che-tauri

The first crates.io candidate is **0.1.2**. It requires **che-orm 0.3.3**;
feature `sync` also requires **che-sync 0.1.2**, which depends on
**che-sync-protocol 0.1.0**. The license is MIT. The independent
`examples/todo-sync` workspace and nested `che-back` repository are excluded.

## Verify

```sh
cargo fmt --check
cargo check --all-targets --all-features
cargo test
cargo test --features sync
cargo clippy --all-targets --features sync -- -D warnings
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
versions: che-orm 0.3.3, che-sync 0.1.2, then che-tauri 0.1.2.
che-sync-protocol 0.1.0 is already published. After the dependencies are indexed:

```sh
cargo publish --dry-run --locked
cargo publish --locked
```

The Git release tag is `v0.1.2`. A tag identifies the candidate and does not imply
that a crates.io publication or docs.rs build has completed.

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
