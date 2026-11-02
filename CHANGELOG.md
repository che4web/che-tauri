# Changelog

## 0.1.2

- Add a typed application command registry and generate TypeScript contracts from
  Serde schemas through `schemars`.
- Add optional Stronghold credential storage and generate vault helpers with
  system-keyring automatic unlock while keeping secrets out of WebView storage.
- Require che-orm 0.3.3 and che-sync 0.1.2, matching the current native consumers.
- Add MIT licensing and crates.io package metadata; exclude the independent
  Todo example and nested development repository from the library archive.

## 0.1.1

- Align ORM resources, serializers and module lifecycle with che-orm and che-rest.
- Add optional protocol-2 and explicit protocol-3 synchronization, including the
  shared Todo example and object-access workflows.
