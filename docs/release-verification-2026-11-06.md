# che-tauri 0.1.2: verification on 2026-11-06

## Status

Functional/package preparation is complete; security clearance is blocked by
five upstream RustSec advisories in the optional `credentials` dependency chain.
No audit exclusions, feature removals or publication were performed.
The crates.io API returned 404 for che-tauri on this date: 0.1.2 remains its
first publication candidate. Preserve the existing v0.1.2 tag (earlier candidate);
use a new `che-tauri-v0.1.2` tag only after the reviewed publication.

The candidate requires che-orm 0.3.3 and optional che-sync 0.1.3. The Todo
example uses che-rest 0.3.4. Core ORM and sync-protocol 0.1.0 are already
published. A repeat crates.io check during preparation confirmed sync 0.1.3
is now indexed and not yanked; the ordinary che-tauri registry dry run passed. REST is an application/example dependency, not a che-tauri library
dependency. No new database migration or automatic protocol change is added.

## Build and integration checks

- Default and all-feature tests passed, including typed command generation,
  credentials helpers, resource behavior and synchronization.
- All-target/all-feature compilation, Clippy with warnings denied, formatting
  and API documentation passed on the installed stable Rust 1.98.0.
- Rust 1.88 was tried and rejected by the Tauri 2 dependency MSRV. Rust 1.90
  all-target/all-feature compilation passed; the manifest now declares 1.90.
- The independent Todo workspace passed desktop compilation, server/client
  tests without default features, Clippy, formatting and JavaScript syntax.
- Compatible root/example lock-file updates fixed the old Tokio, TLS and QUIC
  findings. The updated Todo audit reports zero vulnerabilities, with inherited
  GTK dependency warnings still present.
- Six sibling archives (ORM/macros/protocol/sync/REST/Tauri) packaged together;
  the extracted Tauri archive passed all-target/all-feature compilation and
  all-feature tests. Library contents exclude che-back, application examples,
  credentials, runtime databases, uploads and build outputs.

Logs and machine-readable results are retained at:

`/home/che/web/rust/tacit/target/tauri-prep-20261106/`

The archive report and hashes are in `packages/result.json`; the candidate
archive is `packages/source/target/package/che-tauri-0.1.2.crate`. The verifier
uses a source snapshot; packaging from a committed repository can change VCS
metadata/hashes. Linux compilation is not a manual GUI, Android or keyring check.

## Unresolved audit findings

The root lock-file audit still exits unsuccessfully with five vulnerabilities
and inherited GTK warnings. This does not mean the audit passed simply because
all functional tests did.

| Dependency | Advisories | Patched branch |
| --- | --- | --- |
| crypto-bigint 0.5.5 | RUSTSEC-2026-0387 | >=0.7.6 |
| zip 8.6.0 | RUSTSEC-2026-0367, RUSTSEC-2026-0368, RUSTSEC-2026-0369, RUSTSEC-2026-0429 | >=9.0.0 |

`cargo tree --all-features` traces crypto-bigint through elliptic-curve/k256,
iota-crypto, iota_stronghold and tauri-plugin-stronghold 2.4.0. The ZIP dependency
is build-time: libsodium-sys-stable 1.24.3 -> stronghold-runtime ->
stronghold_engine -> iota_stronghold -> tauri-plugin-stronghold.
Both paths enter che-tauri through optional `credentials`.

Compatible `cargo update` was run; current upstream requirements still retain
these branches. A lock-only update cannot move those dependencies to the patched
incompatible major versions. Do not force an incompatible dependency override,
disable credential storage, or claim these findings are unreachable without a
review. Follow-up requires compatible upstream fixes or an explicit reviewed
resolution of these findings before treating the candidate as security-cleared.

Primary advisories:

- [crypto-bigint timing variability](https://rustsec.org/advisories/RUSTSEC-2026-0387.html)
- [ZIP permission/extraction issue](https://rustsec.org/advisories/RUSTSEC-2026-0369.html)
- [ZIP advisory index](https://rustsec.org/packages/zip.html)

## Publication gate

After resolving/reviewing the security findings, review and commit the intended
source, then repeat the dry run from a clean library worktree. The dependency
indexing gate is already satisfied; no publication was performed here:

```sh
cargo publish --dry-run --locked
cargo publish --locked
```

Verify the crate page and docs.rs before creating the new package-specific tag.
Preparation does not publish, accept audit findings or modify the old tag.
