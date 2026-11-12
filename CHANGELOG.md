# Changelog

## 0.2.0 (unreleased)

- Require che-orm 0.4.0 and che-sync 0.2.0 and adopt bound-query execution.
- Preserve the 0.1.2 credential/configuration and dependency updates when merging
  the parallel main histories. Existing SQLite apps require explicit
  application-owned datetime normalization before opening the new runtime.

## 0.1.2

- Add a typed application command registry and generate TypeScript contracts from
  Serde schemas through `schemars`.
- Add optional Stronghold credential storage and generate vault helpers with
  system-keyring automatic unlock while keeping secrets out of WebView storage.
- Require che-orm 0.3.3 and che-sync 0.1.3 for the current native consumers,
  including optional batched model journal pulls through supplied transports.
- Upgrade configuration parsing from toml 0.8 to toml 1.2 and align direct
  dependency requirements with the tested current stable versions. Todo keeps
  reqwest 0.12 for the che-rest HTTP transport.
- Declare Rust 1.90 as the minimum supported compiler for current Tauri 2.
- Refresh compatible dependencies to fix Tokio, TLS and QUIC advisories;
  document remaining upstream Stronghold/ZIP findings without audit exclusions.
- Add MIT licensing and crates.io package metadata; exclude the independent
  Todo example and nested development repository from the library archive.

## 0.1.1

- Align ORM resources, serializers and module lifecycle with che-orm and che-rest.
- Add optional protocol-2 and explicit protocol-3 synchronization, including the
  shared Todo example and object-access workflows.
