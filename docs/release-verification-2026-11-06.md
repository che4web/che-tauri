# che-tauri 0.1.2: verification on 2026-11-06

## Status

The native Stronghold adapter and `credentials` Cargo feature were removed
at the user's request. The library RustSec audit now reports zero vulnerabilities
(inherited GTK warnings remain). `Management::with_credentials()` and the generated
vault/system-keyring helper are unchanged; consuming applications own native
plugin setup. No audit exclusions or publication were performed.
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

## Former library audit findings (still relevant to Stronghold consumers)

Before removing the native adapter, the root lock-file audit reported five
vulnerabilities. These findings still apply to applications that directly use
the same Stronghold dependency chain; they were not fixed in upstream crates.

| Dependency | Advisories | Patched branch |
| --- | --- | --- |
| crypto-bigint 0.5.5 | RUSTSEC-2026-0387 | >=0.7.6 |
| zip 8.6.0 | RUSTSEC-2026-0367, RUSTSEC-2026-0368, RUSTSEC-2026-0369, RUSTSEC-2026-0429 | >=9.0.0 |

Before removal, `cargo tree --all-features` traced crypto-bigint through elliptic-curve/k256,
iota-crypto, iota_stronghold and tauri-plugin-stronghold 2.4.0. The ZIP dependency
is build-time: libsodium-sys-stable 1.24.3 -> stronghold-runtime ->
stronghold_engine -> iota_stronghold -> tauri-plugin-stronghold.
Both paths entered che-tauri through its former optional `credentials` feature.

Compatible `cargo update` was run; current upstream requirements still retain
these branches. A lock-only update cannot move those dependencies to the patched
incompatible major versions. Do not force an incompatible dependency override,
disable credential storage, or claim these findings are unreachable without a
review. For applications retaining Stronghold, resolution requires compatible upstream
fixes or an explicit reviewed treatment of these findings.

Primary advisories:

- [crypto-bigint timing variability](https://rustsec.org/advisories/RUSTSEC-2026-0387.html)
- [ZIP permission/extraction issue](https://rustsec.org/advisories/RUSTSEC-2026-0369.html)
- [ZIP advisory index](https://rustsec.org/packages/zip.html)

## Publication gate

After reviewing the native adapter removal, commit the intended source and
repeat the dry run from a clean library worktree. The dependency
indexing gate is already satisfied; no publication was performed here:

```sh
cargo publish --dry-run --locked
cargo publish --locked
```

Verify the crate page and docs.rs before creating the new package-specific tag.
Preparation does not publish, accept audit findings or modify the old tag.


## Dependency refresh before native-adapter removal

Direct dependency requirements now use the tested current stable baselines:
Tauri 2.12.5, Stronghold plugin 2.4.0, reqwest 0.13.5, Tokio 1.54.1,
schemars 1.2.3 and current serde/clap/async-trait/thiserror patches. These are
ordinary caret requirements, allowing future compatible updates.
Configuration parsing was upgraded from toml 0.8 to toml 1.2.0; file-loading
tests cover database-only and remote settings, literal paths, missing fields
and invalid types. The Todo transport retains reqwest 0.12.28 for compatibility
with che-rest; its other direct requirements are aligned with current versions.

After this refresh, all-feature tests, Rust 1.90 all-target/all-feature checks,
Clippy with warnings denied, Todo desktop compilation and server/client tests
passed. A new all-feature registry publish dry run also passed; no upload was
performed. These logs have the `update-` prefix in the verification directory.
The six-archive report above describes the earlier preparation snapshot; it
must not be treated as the hash report for these subsequent source edits.

At that stage root audit still reported five Stronghold-chain advisories;
Todo reports zero vulnerabilities. At that stage no alpha plugin upgrade, incompatible crypto/ZIP override,
feature removal or advisory ignore was introduced.

The Todo graph still contains transitive toml 0.8.2: GTK's proc-macro-crate 2.0.2
requires toml_datetime exactly 0.6.3. Forcing toml 0.8.23 was tried and rejected
by Cargo because it needs toml_datetime >=0.6.11. Retain the compatible resolver
result until upstream constraints change; che-tauri's own parser is now 1.2.


## Native Stronghold adapter removal

The Rust library no longer has a tauri-plugin-stronghold dependency or native
credentials feature/installer. Cargo pruned crypto-bigint 0.5.5, ZIP 8.6.0 and
the rest of that chain from the library lock file; its RustSec audit reports
zero vulnerabilities. This removes the chain from the library rather than
patching upstream cryptography or ZIP code. Known GTK warnings remain.

The generated CredentialVault/SystemCredentialStore helper stays intact. Apps
choosing it install their native plugin directly and own its dependency audit.
Tacit keeps its direct native Stronghold dependency: its setup now initializes
the same builder with the same app-local vault-salt path. Existing vault files,
namespace, account identity and system-keyring entry are not migrated or deleted.
The mobile app's Stronghold advisories are still present; this change does not
claim to resolve them. Native compilation and offline tests validate integration,
not a fresh device install or GUI/keyring session.

Current checks have the `no-stronghold-` prefix in the verification directory.
The earlier six-archive report is a historical snapshot, not the current
candidate's hash report. Package preparation/dry runs do not publish the crate.
