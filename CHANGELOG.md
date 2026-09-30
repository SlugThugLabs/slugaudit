# Changelog

All notable changes to SlugAudit are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
The newest entry MUST match the version in `Cargo.toml`.

## [Unreleased]

<!-- Entries for the next release go here. -->

## [1.0.10] - 2026-09-06

Release commit `release: bump version to 1.0.10 for hardware-adaptive scaling
and profile release`, tagged `v1.0.10` on 2026-09-06. `Cargo.toml` records
`version = "1.0.10"`.

### Added

- Hardware-adaptive scaling and the `adaptive` / `audit` / `lean` resource
  profiles (`src/model/profile.rs`, `src/model/limits.rs`).

[Unreleased]: https://github.com/SlugThugLabs/slugaudit/compare/v1.0.10...HEAD
[1.0.10]: https://github.com/SlugThugLabs/slugaudit/releases/tag/v1.0.10
