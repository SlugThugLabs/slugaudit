# Releasing SlugAudit

This document describes the release workflow
(`.github/workflows/release.yml`), the artifacts it publishes, and what a
consumer can and cannot conclude from each one. It is written for the person
cutting a release and for the person verifying a download.

The shipped product is the single `slugaudit` binary. The `check_*` binaries
under `src/bin/` are development/CI tools and are not distributed.

## Scope and trigger

The workflow triggers on:

- a push of a tag matching `v*` (the normal path), or
- a manual `workflow_dispatch` with a `tag` input.

The checkout uses that same tag (`RELEASE_TAG`), not the branch the workflow
was dispatched from, so the artifact is built from the source the tag
describes. Bump `Cargo.toml`'s version to match the tag before cutting the
release.

**Platform boundary:** releases are **Linux x86_64 only**
(`x86_64-unknown-linux-gnu`). That is the platform CI verifies. Runtime
support is Linux and macOS, but no macOS release asset is published; macOS
users build from source (`cargo install --path .`, or the installer's cargo
fallback). `slugaudit update` refuses to fetch the Linux asset on any other
platform.

The build runs on `ubuntu-latest` with Rust **1.97.1** pinned in
`rust-toolchain.toml`, and builds with:

```bash
cargo build --release --locked --bin slugaudit
```

`--locked` makes the build fail rather than silently resolve a different
dependency set than `Cargo.lock`.

## Jobs

### `build-linux-x86_64`

Permissions: `contents: write` (create the release and upload assets),
`id-token: write` (OIDC token for the attestation), `attestations: write`
(store the signed attestation). The workflow-level default is `contents:
read` only; every other job inherits read.

Steps:

1. Check out `RELEASE_TAG`.
2. Install the pinned toolchain (1.97.1).
3. Cache cargo registry/build artifacts (`Swatinem/rust-cache`).
4. Build the release binary.
5. Stage `dist/slugaudit-x86_64-unknown-linux-gnu`, copy `LICENSE`, and
   generate `SHA256SUMS` over the binary only.
6. Generate a CycloneDX 1.5 JSON SBOM with
   `cargo cyclonedx --format json --spec-version 1.5 --override-filename
   slugaudit-x86_64-unknown-linux-gnu.cdx`.
7. Generate `THIRD-PARTY-NOTICES.txt` with cargo-about 0.9.2.
8. Attest build provenance for the binary.
9. Upload every file under `dist/` to the GitHub Release, with a release body
   that repeats the consumer verification commands.

`RUSTFLAGS` remaps two build paths to fixed prefixes:

```text
--remap-path-prefix=${{ github.workspace }}=/slugaudit
--remap-path-prefix=/home/runner/.cargo=/cargo
```

The remapping exists because a build records absolute paths inside the
binary (the released 1.0.10 asset carried 419 cargo-registry paths plus one
workspace path). Mapping them to fixed prefixes makes those recorded paths a
function of the source rather than of where the tree sat. It is a
reproducibility aid, not a proof; see *rebuild-and-compare* below.

Every third-party `uses:` is pinned to the full commit SHA behind its release
tag, with the version as a trailing comment. `dtolnay/rust-toolchain`
publishes no release tags, so it is re-pinned by hand. `.github/dependabot.yml`
proposes the next pin weekly for both `cargo` and `github-actions`.

### `rebuild-and-compare` (best effort)

This job depends on `build-linux-x86_64` but the release job does not depend
on it: nothing here blocks or unpublishes a release. It:

1. checks out the released tag,
2. installs the same pinned toolchain,
3. deliberately uses **no cache** (a cache hit could hand it the other job's
   `target/` and compare a build with itself),
4. downloads the published asset,
5. rebuilds with the same `--locked` command and the same `RUSTFLAGS` into the
   same default target path,
6. compares the whole files, then compares sections with `objcopy`.

What it compares is decided from measurement, not assumption. Two builds of
this dependency graph are never byte-identical because `tree-sitter-language-pack`
uses `ahash` with `compile-time-rng`, which pulls in `const-random` and embeds
a random constant at compile time. Consequently:

- **Volatile sections** (reported, not a failure): `.rodata` (holds the
  random constant), `.strtab` (moves with LLVM's `llvm.<hash>` symbol
  suffixes), and `.note.gnu.build-id` (a hash of the output).
- **Compiled-code sections** (a difference fails the job): `.text`, `.plt`,
  `.data`, `.data.rel.ro`, `.eh_frame`, `.gcc_except_table`, `.rela.dyn`,
  `.rela.plt`, `.dynsym`, `.dynstr`, `.init_array`, `.fini_array`.

If the whole-file digests match, the job reports bit-identical and passes. If
they differ, it reports both digests, the first differing byte, and the
section-by-section result; it passes only when every compiled-code section is
identical and fails if any differs or if `objcopy` is unavailable. A green run
means the compiled code is reproducible from the tag in that environment. It
does **not** mean the artifact is bit-for-bit reproducible, and it does not
cover the SBOM, the notices, or `LICENSE`.

## Published artifacts

| File | What it is | What it proves |
|---|---|---|
| `slugaudit-x86_64-unknown-linux-gnu` | The release binary. | — |
| `SHA256SUMS` | One line: SHA-256 and filename for the binary. | Integrity of the download relative to this release page. |
| `LICENSE` | PolyForm Noncommercial 1.0.0 plus the SlugAudit Tool-Use Additional Permission. | The terms the binary is licensed under. |
| `slugaudit-x86_64-unknown-linux-gnu.cdx.json` | CycloneDX 1.5 SBOM, JSON. | The dependency graph the binary was built from. |
| `THIRD-PARTY-NOTICES.txt` | Every third-party crate in that graph, with its licence text. | The notices the licence requires to travel with a copy. |
| Build-provenance attestation | Sigstore-signed SLSA v1 statement, stored on the repository. | The binary's digest was produced by this workflow from this tag on a GitHub-hosted runner. |

### `SHA256SUMS` is integrity, not authenticity

The manifest is published from the same origin as the binary. It proves the
download was not corrupted or truncated in transit and that it matches what
the release page lists. It cannot prove the artifact is genuine: whoever could
replace the binary on the release could replace the manifest beside it just as
easily. The format is load-bearing: `install.sh` and `slugaudit update` both
read this file, and it covers the binary only.

### The attestation is what carries authenticity

The build job uses `actions/attest-build-provenance` (v4) before uploading,
so a release that exists without an attestation is not produced. The
attestation is a Sigstore-signed statement binding the binary's SHA-256 digest
to the workflow and source commit that produced it. `gh attestation verify`
checks that binding. It does **not** say the code is correct, safe, or free of
the vulnerabilities you care about, and it covers the binary only — not the
SBOM, the notices, or `LICENSE`. `create-storage-record` is off because the
release publishes a release asset, not an OCI image.

### SBOM caveats

The SBOM is generated from `Cargo.lock` by `cargo cyclonedx` 0.5.9 at spec
version 1.5, for the default feature set. Its root component is the crate
(`slugaudit-mcp-rust`); `slugaudit` is its only shipped product.
Dev-dependencies are absent because a release build never compiles them.
The document carries a generation timestamp and the building machine's
manifest path, so it is not itself reproducible; it is not part of the
`rebuild-and-compare` comparison and it is not attested.

### Third-party notices

`THIRD-PARTY-NOTICES.txt` is generated from the tag's own dependency graph;
there is no checked-in copy to go stale. The command is:

```bash
cargo about generate --locked --fail -c docs/about.toml \
  -o "$RUNNER_TEMP/notices.body" docs/third-party-notices.hbs
```

`--fail` turns an undeterminable licence into a failed release instead of a
silently missing section. The workflow prefixes the generated body with the
release tag, commit, artifact name, the generator version, and the root
crate's own terms. `LICENSE`'s Notices clause requires whoever receives a copy
to also receive the licensor's `Required Notice:` lines, so the header quotes
the first `Required Notice:` line from `LICENSE` verbatim. SlugAudit itself is
not third-party and is not listed in the body.

`docs/about.toml` holds the cargo-about configuration; its accepted-licence
list mirrors `deny.toml`'s `[licenses] allow` list. The fuzz workspace is a
separate cargo workspace and is not part of this release graph, so its
development-only `libfuzzer-sys` NCSA exception does not appear here.
`docs/third-party-notices.hbs` is the Handlebars template. If either file is
missing or invalid, the release job fails rather than publishing a release
without notices.

The template was validated by a full render through the workflow command with
`cargo-about 0.9.2` (the workflow's version): 6,558 lines / 340,298 bytes,
exit 0, no warnings. Two output defects found in that render are fixed and
must stay fixed. License names use the raw `{{{name}}}` expression, so a name
containing `&` or `"` is not emitted as an HTML entity (`&quot;`). The
template also no longer writes cargo-about's `source_path` block, so
machine-specific registry paths (`/root/.cargo/...`, `/home/runner/...`) do
not ship to consumers.

One limitation is upstream and deliberately not patched around: `ring`
0.17.14's ISC licence text is extracted from source-file headers and carries
trailing code fragments into the notices. `ring`'s `LICENSE` points only to
`LICENSE-other-bits`, so cargo-about falls back to scanning the sources; the
fragment is cargo-about behavior, not a template defect. Do not hand-edit the
generated file — a template change would not remove it.

## Verifying a release

The release body repeats these four commands.

```bash
# 1. Integrity: the bytes you have are the bytes this release published.
sha256sum --check --ignore-missing SHA256SUMS

# 2. Provenance: this binary was built by this repository's release workflow,
#    from this tag's source, on a GitHub-hosted runner. (gh 2.49 or newer)
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

What each command buys you:

1. **Integrity.** The manifest and binary are from the same origin. This
   catches corruption, truncation, and a mismatch with the release page; it
   does not prove authenticity.
2. **Authenticity.** The attestation binds the digest to this repository's
   release workflow and the tag's source commit, and `--deny-self-hosted-runners`
   rejects a run from a self-hosted runner. It does not evaluate the code.
3. **Dependency graph.** The SBOM names the components the binary was built
   from. It is a build record, not a vulnerability verdict.
4. **Terms.** `LICENSE` is the binding text; the notices file carries the
   third-party licence texts the release distributes.

A release with no attestation is not a release this workflow produces; if
`gh attestation verify` cannot find one, treat the download as unverified.

## Release checklist

1. Bump `Cargo.toml`'s version (currently `1.0.10`) and update
   `CHANGELOG.md` / `lhm.plugin.json` as required by
   `check_version_consistency`.
2. Confirm `src/parse/mod.rs`'s `PACK_VERSION` matches the exact
   `tree-sitter-language-pack` pin in `Cargo.toml`.
3. Run the local gates in `CONTRIBUTING.md`; the release itself runs
   `cargo build --release --locked`.
4. Commit `Cargo.lock` with the manifest change; the release builds
   `--locked`.
5. Push a `v*` tag (or dispatch the workflow with the tag). The checkout,
   upload, and release body all use that one tag.
6. Verify the release body's four commands against a downloaded artifact.
7. Treat a failing `rebuild-and-compare` as a report, not a release blocker:
   it does not affect the published, checksummed, attested bytes, but it does
   mean reproducibility from the tag was not demonstrated. Compare the runner
   image, toolchain, and `RUSTFLAGS` printed in that job before assuming a
   code problem.

## What the release does not cover

- Windows and macOS binaries: none are built or published.
- Container images: none.
- The SBOM, notices, and `LICENSE` are documents; only the binary is
  attested.
- `SHA256SUMS` does not establish who built the artifact.
- A green `rebuild-and-compare` does not establish bit-for-bit reproducibility
  (the `.rodata` random constant and build-id necessarily differ).
