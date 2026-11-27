# Releasing SlugAudit

This document describes `.github/workflows/release.yml`, the assets it
publishes, and how to verify a downloaded release.

The only distributed executable is `slugaudit`. The `check_*` binaries in
`crates/gates` (`slugaudit-gates`) are development/CI gates and are never
packaged or shipped.

## Trigger and platform scope

The release workflow runs on:

- a tag push matching `v*` (for example `v1.1.0`), or
- a manual `workflow_dispatch` with a `tag` input.

Both jobs check out `ref: ${{ env.RELEASE_TAG }}` rather than the branch the
workflow was dispatched from, and `Verify Cargo.toml version matches release tag`
fails the build unless `v<Cargo.toml version>` equals `RELEASE_TAG` (currently
`1.1.0` in `Cargo.toml` and `lhm.plugin.json`).

**Platform scope:** releases publish **Linux x86_64**
(`x86_64-unknown-linux-gnu`) built on `ubuntu-latest` with Rust **1.97.1** via:

```sh
cargo build --release --locked --bin slugaudit
```

Both jobs compile with fixed path remapping so build-machine workspace and Cargo
registry paths are not embedded verbatim:

```text
RUSTFLAGS="--remap-path-prefix=${GITHUB_WORKSPACE}=/slugaudit --remap-path-prefix=/home/runner/.cargo=/cargo"
```

macOS and other platforms build from source (`install.sh`'s `cargo` fallback or
`TSLP_LINK_MODE=static TSLP_LANGUAGES=rust cargo install --git https://github.com/SlugThugLabs/slugaudit.git --locked --bin slugaudit`).
`slugaudit update` refuses non-Linux-x86_64 hosts.

## Published release assets

Each GitHub Release attaches five files from `dist/` plus one repository-stored
provenance attestation:

| Asset | Produced by | Purpose |
|---|---|---|
| `slugaudit-x86_64-unknown-linux-gnu` | `cargo build --release --locked --bin slugaudit` | Shipped MCP server and CLI binary. |
| `SHA256SUMS` | `sha256sum slugaudit-x86_64-unknown-linux-gnu` | SHA-256 digest of the binary (also read by `install.sh` and `slugaudit update`). |
| `LICENSE` | Copied from `LICENSE` | PolyForm Noncommercial License 1.0.0 plus the SlugAudit Tool-Use Additional Permission. |
| `THIRD-PARTY-NOTICES.txt` | `cargo-about 0.9.2` (`docs/about.toml`, `docs/third-party-notices.hbs`) + `release.yml` static C grammar appendix | Required notice line from `LICENSE`, licence texts of every third-party Cargo crate compiled into the Linux release binary, and the explicit MIT licence attribution for statically compiled `tree-sitter-rust`. |
| `slugaudit-x86_64-unknown-linux-gnu.cdx.json` | `cargo-cyclonedx 0.5.9` (`--spec-version 1.5`) | CycloneDX 1.5 JSON SBOM of the release dependency graph. |
| Build provenance attestation | `actions/attest-build-provenance` | Sigstore-signed SLSA v1 provenance binding the binary's SHA-256 digest to `release.yml` and `RELEASE_TAG`. |

## Verifying a release

Every GitHub Release body includes these four commands:

```sh
# 1. Integrity: the bytes you have are the bytes this release published.
sha256sum --check --ignore-missing SHA256SUMS

# 2. Provenance: this binary was built by this repository's release
#    workflow, from this tag's source, on a GitHub-hosted runner.
#    (needs gh 2.49 or newer)
gh attestation verify slugaudit-x86_64-unknown-linux-gnu \
  --repo SlugThugLabs/slugaudit \
  --signer-workflow SlugThugLabs/slugaudit/.github/workflows/release.yml \
  --deny-self-hosted-runners

# 3. The SBOM: the dependency graph the binary was built from.
jq -r '.metadata.component | "\(.name) \(.version)"' \
  slugaudit-x86_64-unknown-linux-gnu.cdx.json

# 4. The terms it ships under.
less LICENSE THIRD-PARTY-NOTICES.txt
```

What each step establishes (and its boundary):

1. **Integrity (`sha256sum --check`).** Confirms the downloaded binary was not
   truncated or corrupted in transit and matches `SHA256SUMS`. Because both
   files come from the same release page, `SHA256SUMS` alone does not prove who
   built the binary.
2. **Provenance (`gh attestation verify`).** Verifies the Sigstore-signed SLSA
   v1 attestation binding the binary digest to
   `SlugThugLabs/slugaudit/.github/workflows/release.yml` and the tagged source
   commit on a GitHub-hosted runner (`--deny-self-hosted-runners`). Only the
   binary is attested; `LICENSE`, `THIRD-PARTY-NOTICES.txt`, and the `.cdx.json`
   SBOM are accompanying documents. If no attestation is found, treat the binary
   as unverified.
3. **Dependency record (`.cdx.json` SBOM).** Lists the resolved Cargo dependency
   graph compiled into the release binary (excluding dev-dependencies and the
   separate `fuzz/` workspace). It embeds a generation timestamp and build path,
   so it is a build record rather than a reproducible artifact or vulnerability
   verdict.
4. **Licensing (`LICENSE` and `THIRD-PARTY-NOTICES.txt`).** `LICENSE` states
   SlugAudit's own terms; `THIRD-PARTY-NOTICES.txt` carries the third-party
   Cargo crate licences verified against `docs/about.toml` (which mirrors
   `deny.toml`'s `[licenses] allow` list) plus an explicit MIT licence notice
   for `tree-sitter-rust` (compiled statically via `tree-sitter-language-pack`'s
   C build script outside Cargo's Rust crate graph).

## Rebuild comparison (`rebuild-and-compare`)

After `build-linux-x86_64` publishes the release, the `rebuild-and-compare` job
checks out `RELEASE_TAG` on a fresh `ubuntu-latest` runner with **no** Cargo
cache, rebuilds `target/release/slugaudit` with the same toolchain and
`RUSTFLAGS`, downloads the published asset, and compares ELF sections using
`objcopy --dump-section <section>=<out> <elf> /dev/null`:

- **Compiled-code sections (must match byte-for-byte):** `.text`, `.plt`,
  `.data`, `.data.rel.ro`, `.eh_frame`, `.gcc_except_table`, `.rela.dyn`,
  `.rela.plt`, `.dynsym`, `.dynstr`, `.init_array`, `.fini_array`.
- **Expected per-build volatile sections (reported, non-fatal):** `.rodata`,
  `.strtab`, and `.note.gnu.build-id`. `tree-sitter-language-pack` pulls `ahash`
  with `compile-time-rng` (`const-random`), which embeds a fresh compile-time
  random constant in `.rodata` on every build and shifts internal `.strtab`
  symbol suffixes and the ELF build-id hash accordingly.

`rebuild-and-compare` runs after publication (`needs: build-linux-x86_64`) as a
diagnostic verification report: a failure there does not alter the published,
attested asset, and indicates that compiled code diverged across runners
(for example a runner-image linker update) so the printed toolchain, `RUSTFLAGS`,
and runner image versions should be inspected.

## Release checklist

1. Confirm `Cargo.toml` (`[package] version`, currently `1.1.0`), `Cargo.lock`,
   and `lhm.plugin.json` (`"version"`) carry the intended version, and that
   `src/parse/mod.rs`'s `PACK_VERSION` matches the `=x.y.z`
   `tree-sitter-language-pack` pin in `Cargo.toml`.
2. Run the full local verification suite from the repository root:
   - `cargo fmt --all --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test --workspace --all-targets`
   - `cargo run -q -p slugaudit-gates --bin check_source_limits`
   - `cargo run -q -p slugaudit-gates --bin check_no_duplicates`
   - `cargo run -q -p slugaudit-gates --bin check_producer_coverage`
   - `cargo run -q -p slugaudit-gates --bin check_mcp_surface`
3. Push the `v<version>` tag (for example `v1.1.0`) or dispatch `release.yml`
   with `tag: v<version>`.
4. Download the published release assets and run the four verification commands
   above, and check the `rebuild-and-compare` job log.
5. Update external MCP directory listings (Glama, LHM, and peer directories) on
   the same day so published metadata matches the tagged code.
