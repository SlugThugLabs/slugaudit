# Releases: what a tag publishes, and how to verify it

Two audiences, one page. If you **downloaded** a release, start at §2 — the
commands there are the whole verification story, and each one says what it
proves and what it does not. If you are **cutting** the release, §4 and §5 are
yours.

SlugAudit reports evidence and states facts; it assigns no severity and renders
no verdict. This page follows the same rule: nothing here is described as
"safe", "verified" or "trustworthy" beyond the exact property the command
actually checks.

## 1. What a release contains

`.github/workflows/release.yml` runs on a `v*` tag push and attaches these to
the GitHub Release:

| Asset | Produced by | What it is |
|---|---|---|
| `slugaudit-x86_64-unknown-linux-gnu` | `cargo build --release --locked --bin slugaudit` | the shipped binary (Linux x86_64 only) |
| `SHA256SUMS` | `sha256sum` | one line: the SHA-256 of the binary |
| `LICENSE` | this repository | the terms the binary is licensed under |
| `slugaudit-x86_64-unknown-linux-gnu.cdx.json` | `cargo-cyclonedx 0.5.9` | CycloneDX 1.5 SBOM of the dependency graph the binary was built from |
| `THIRD-PARTY-NOTICES.txt` | `cargo-about 0.9.2` | every third-party crate in that graph, with the licence text each crate ships |

One more thing is produced but has no file: a **build-provenance attestation**
for the binary, signed through Sigstore and stored on the repository (not in the
release's asset list, because the GitHub CLI fetches it by digest).

`SHA256SUMS` still covers the binary alone. Its format is what `install.sh` and
`slugaudit update` parse (`src/update.rs::verify_checksum` matches the entry by
filename), so it did not grow when the other assets were added.

## 2. Verifying a download

Download the binary (at minimum) and run the commands in order. Each subsection
states the property that command establishes and the property it does not.

### 2.1 The checksum — integrity, not authenticity

```sh
sha256sum --check --ignore-missing SHA256SUMS        # GNU coreutils
shasum -a 256 --check --ignore-missing SHA256SUMS    # macOS
```

**Proves:** the bytes on your disk hash to the digest the release published. A
truncated download, a corrupted transfer, or a mirror serving a different file
fails here.

**Does not prove:** that the file is genuine. The manifest and the binary are
published from the same origin, so whoever could replace one could replace the
other — a checksum is what makes a download *complete*, not what makes it
*trustworthy*. **`SHA256SUMS` from the same origin as the artifact is integrity,
not authenticity.** That is precisely the gap the attestation in §2.2 exists to
close; if you run only one of these two commands, run that one.

### 2.2 The attestation — authenticity

```sh
gh attestation verify slugaudit-x86_64-unknown-linux-gnu \
  --repo SlugThugLabs/slugaudit \
  --signer-workflow SlugThugLabs/slugaudit/.github/workflows/release.yml \
  --deny-self-hosted-runners
```

Your own `./slugaudit` binary works too — `gh attestation verify ./slugaudit`
looks up attestations by the file's digest, so it verifies *that file*, not a
name. Requires `gh` 2.49.0 or newer (the release in which `gh attestation`
shipped) and, for the API calls, an authenticated session (`gh auth login`).

**Proves:** a Sigstore-signed statement binding your file's SHA-256 digest to
the workflow that produced it. The certificate's identity comes from GitHub's
OIDC issuer for the run, so `--repo` + `--signer-workflow` pin *which repository
and which workflow file* signed it, and `--deny-self-hosted-runners` pins that
it ran on a GitHub-hosted runner. `gh attestation verify` enforces the SLSA
build-provenance predicate (`https://slsa.dev/provenance/v1`) by default, so the
statement's source repository, commit and ref are checked too. Add
`--source-ref refs/tags/<TAG>` to require that the workflow ran on that tag
rather than a branch — true for a normal tag-push release, and false for one cut
by manual dispatch, which records the branch it was dispatched from.

**Does not prove:** anything about the quality, safety or desirability of the
code, and nothing about the dependencies. Two limits worth stating plainly,
both from `gh attestation verify`'s own documentation:

- Only the certificate identity and the transparency-log timestamps are
  unforgeable by the workflow. The *predicate* — the source repository, commit
  and ref the statement names — is written by the workflow itself, so an
  attacker who has taken over the workflow's execution context could sign a
  false statement. What the attestation rules out is a file that was never built
  by this workflow at all.
- It covers the **binary only**. The SBOM, `THIRD-PARTY-NOTICES.txt` and
  `LICENSE` are documents; no attestation is made about them, and `gh
  attestation verify` on them verifies nothing.

### 2.3 The SBOM — the dependency graph, not the binary

```sh
# The root component, and how many components were recorded.
jq -r '.metadata.component | "\(.name) \(.version)"' slugaudit-x86_64-unknown-linux-gnu.cdx.json
jq -r '.metadata.component, (.components[] | .name + " " + .version)' \
  slugaudit-x86_64-unknown-linux-gnu.cdx.json | head
```

**Proves:** which crates — with exact versions — the release build resolved from
`Cargo.lock` for `x86_64-unknown-linux-gnu`, plus the build-time dependencies of
those crates. It is generated from the manifest and lockfile at release time.

**Does not prove:** that the shipped binary actually contains those components.
`cargo-cyclonedx` records the dependency graph; it does not scan the compiled
artifact, so it is not a substitute for inspecting a binary. It also carries a
generation timestamp and the build machine's manifest path in the root
component's `bom-ref`, so it is not byte-reproducible — though neither is the
binary, for a different reason (§4). Nothing attests it.

### 2.4 The notices — the terms and the notices

```sh
less LICENSE THIRD-PARTY-NOTICES.txt
grep -n '^Required Notice:' THIRD-PARTY-NOTICES.txt
```

**Proves:** which crates are in the graph, the licence expression each one
declares, and the licence text each one ships, reproduced verbatim from the
crate's own files. `LICENSE`'s Notices clause is the model: anyone who receives
a copy of SlugAudit gets the terms, the URL for them, and the plain-text
`Required Notice:` lines, and the same file carries it for every third-party
dependency.

**What it does not do:** it is generated by scanning the dependency graph, not
by reading the binary, and it excludes dev-dependencies (a release build never
compiles them) and anything only another target pulls in (the release is Linux
x86_64 only). `cargo-about` identifies licence text by scanning each crate's
files, so a crate that vendors licence text inside a source file can contribute
more than one entry for the same licence id; both texts are reproduced, which
over-attributes rather than under-attributes. `cargo-about` is configured by
`docs/about.toml` and rendered through `docs/third-party-notices.hbs`; the
accepted list there mirrors `deny.toml`'s `[licenses] allow`, because
`cargo-about` has no way to read `deny.toml` and two policies that can disagree
are worse than one.

## 3. What none of this covers

- **The binary's behaviour.** Every command above is about origin, integrity
  and listed components. Nothing here says the program is correct, or free of
  the vulnerabilities you care about.
- **The other assets' provenance.** Only the binary is attested.
- **macOS or non-x86_64 builds.** They do not exist. `release.yml` builds what
  CI verifies; a platform without CI coverage is not shipped from here.
- **The `check_*` binaries.** Development/CI tools; not distributed.

## 4. Reproducibility: best effort, and exactly what the job compares

**The short version: two builds of this tag are never byte-identical, for a
reason that lives in the dependency graph, and the release says so instead of
implying otherwise.** What `rebuild-and-compare` actually verifies is that the
*compiled code* is identical. `SHA256SUMS` and the attestation describe the
bytes that were really published, and nothing about a release depends on this
job — the release job does not wait for it, so a red run here never blocks or
unpublishes anything.

### 4.1 Why the bytes differ

`tree-sitter-language-pack` depends on `ahash` with the `compile-time-rng`
feature, which pulls in `const-random`:

```console
$ cargo tree -i const-random
const-random v0.1.18
└── ahash v0.8.12
    └── tree-sitter-language-pack v1.14.3
        └── slugaudit-mcp-rust v1.0.10 (/…/slugaudit)
```

That macro generates a random constant at compile time and embeds it in the
binary. Measured on 2026-10-03 by building this tag twice on one machine, at one
path, with one toolchain (`1.97.1`) and identical flags:

| ELF section | Two builds of the same tag |
|---|---|
| `.text`, `.plt`, `.data`, `.data.rel.ro`, `.eh_frame`, `.gcc_except_table`, `.rela.dyn`, `.rela.plt`, `.dynsym`, `.dynstr`, `.init_array`, `.fini_array` | byte-identical |
| `.rodata` | differs — the compile-time random constant |
| `.strtab` | differs — LLVM's internal `.llvm.<hash>` suffixes for that crate's symbols move with it |
| `.note.gnu.build-id` | differs — a hash of the output, so it moves with anything |

No build flag makes those three stable; `SOURCE_DATE_EPOCH` would not either. The
only way to remove the first one is to drop the feature — an owner decision with a
real cost (it is a hash-map seed), not something a release job can paper over.

A second, narrower source of divergence: `tree-sitter-language-pack`'s build
script embeds a path under the *target* directory (the v1.0.10 asset contains
exactly one, `/home/runner/work/slugaudit/slugaudit/target/release/build/…`).
That string depends on where the tree and the target directory sit, so two builds
only record the same one when they share both — which the two jobs do, and which
`RUSTFLAGS` (below) makes independent of the registry layout.

### 4.2 What the job does

It checks out the released tag into a **fresh job**, downloads the published
asset back from the release, and rebuilds with the same pinned toolchain and the
same `RUSTFLAGS`:

```
--remap-path-prefix=${{ github.workspace }}=/slugaudit
--remap-path-prefix=/home/runner/.cargo=/cargo
```

The released v1.0.10 asset embeds 419 absolute paths from the cargo registry and
one from the checkout; mapping them to fixed prefixes is what keeps the two
builds from recording different strings for the same inputs. Both jobs inherit
the value from one workflow-level `env`, so they cannot drift apart.

It reuses nothing: no cache step at all, so `target/` starts empty and every
crate below was compiled by this job. (The cache is omitted on purpose —
`Swatinem/rust-cache` restores a whole archive by key, so a fallback key match
could hand this job the other job's `target/` and turn the comparison into a
comparison of one build with itself.)

It prints what it compared before deciding anything: the tag, the commit, the
`RUSTFLAGS` in effect, the runner image (`ImageOS`/`ImageVersion`), `rustc -vV`,
both SHA-256 digests and both file sizes, the first differing byte, the number of
differing bytes, and a per-section verdict for the twelve code sections plus
`.rodata`, `.strtab` and `.note.gnu.build-id`.

### 4.3 What makes it fail

- **A compiled-code section differs** (`.text`, `.plt`, `.data`,
  `.data.rel.ro`, `.eh_frame`, `.gcc_except_table`, `.rela.dyn`, `.rela.plt`,
  `.dynsym`, `.dynstr`, `.init_array`, `.fini_array`). This is the failure the
  job exists for. In the order you should check: a **different runner image**
  (the two jobs run on whatever `ubuntu-latest` pointed at when they ran; the
  linker comes from the image), a **toolchain that moved** (`dtolnay/rust-toolchain`
  installs `1.97.1`; a re-released or yanked build changes the compiler), or a
  **genuine nondeterminism** in a build script or code generator.
- **`objcopy` is missing**, in which case code and metadata cannot be told
  apart and the job fails closed rather than declaring a match it did not
  establish.

It does **not** fail when the only differences are `.rodata`, `.strtab` and
`.note.gnu.build-id` — §4.1 is the measurement that says that is the expected
shape for this graph, and the run emits a warning saying exactly that.

### 4.4 What a green run means, and does not

Green means: *for the same source, toolchain and flags, the compiled code is
byte-identical; the files still differ in the sections the random constant
touches.* It is **not** a claim that the artifact is bit-for-bit reproducible —
it is not, by §4.1, and that will not change without an owner decision on the
`compile-time-rng` feature. It is also not a claim about any other machine: a
rebuild elsewhere differs in the build-script path strings too, because those are
only identical when the target directory and the `RUSTFLAGS` values are.

### 4.5 Rebuilding yourself

```sh
git checkout <TAG>
export RUSTFLAGS="--remap-path-prefix=$PWD=/slugaudit --remap-path-prefix=$CARGO_HOME=/cargo"
cargo build --release --locked --bin slugaudit
```

`sha256sum target/release/slugaudit` will **not** match the published asset, for
the reasons in §4.1 and §4.2 — expect that, and do not read it as a problem with
either build. What you can check is what the job checks:

```sh
for s in .text .data .rela.dyn; do
  objcopy --dump-section "$s=/tmp/asset.$s" slugaudit-x86_64-unknown-linux-gnu /dev/null
  objcopy --dump-section "$s=/tmp/build.$s" target/release/slugaudit /dev/null
  cmp -s "/tmp/asset.$s" "/tmp/build.$s" && echo "$s identical" || echo "$s DIFFERS"
done
```

(`/dev/null` is the required output file: without one, `objcopy` rewrites its
input in place.) The same caveat applies as above — the two sides only match
when they were built at the same path with the same `RUSTFLAGS`, which is what
the CI job arranges and a local build cannot.

## 5. For the release operator

### 5.1 Cutting a release

1. Bump the version in `Cargo.toml` and `lhm.plugin.json` and add the
   `CHANGELOG.md` entry — `check_version_consistency` fails the build otherwise.
2. Push the tag (`v*`). The workflow checks out **that tag** (not the default
   branch, including on a manual `workflow_dispatch` run), builds, generates the
   paperwork, attests, and attaches everything.
3. Read the `rebuild-and-compare` job's log. A `::warning::` and a green job is
   the expected result for a good release: the whole-file digests will differ
   (never bit-for-bit reproducible, §4.1) and every compiled-code section should
   read `identical`. A red job means a code section differed — a finding to
   write down and explain, not a broken release.

The release body is written by the workflow and carries the four verification
commands in short form. It is not generated from the commit log, and
`generate_release_notes` is deliberately left unset — turning it on would
replace this text with the commit list.

### 5.2 Permissions, and the minimum each step needs

The workflow grants `contents: read` at the top level, so every job inherits
read and nothing more. `build-linux-x86_64` is the only job that asks for more:
`contents: write` to create the release and upload assets, `id-token: write` for
the OIDC token the attestation signature is minted for, and `attestations: write`
to store the signed result on the repository. `rebuild-and-compare` needs
`contents: read` and nothing else.

`artifact-metadata: write` is deliberately **not** granted. The attestation
action offers to create a registry storage record, but that record associates an
attestation with an OCI registry artifact and this release publishes release
assets, so `create-storage-record: false` is set explicitly. `actions/attest`
wraps that call in a `try/catch` precisely so the scope can be withheld
(`actions/attest` `src/attest.ts`).

### 5.3 Pinned inputs

Every action is pinned to a full commit SHA (the peeled commit for annotated
tags) with the version in a comment, and `dependabot.yml` proposes the next pin.
The two tools installed by `taiki-e/install-action` are pinned the same way, in
the workflow rather than in `Cargo.toml`, because they are release machinery and
not dependencies of the shipped binary:

| Input | Pin | Why this one |
|---|---|---|
| `taiki-e/install-action` | `v2.87.22` | already used by `quality.yml` and `supply-chain.yml`; native GitHub Release assets with checksum verification, so no compile step and no `cargo-binstall` fallback (`fallback: none`) |
| `cargo-cyclonedx` | `0.5.9` | CycloneDX for Rust, maintained under the CycloneDX org |
| `cargo-about` | `0.9.2` | generates the attribution list from the dependency graph, per this repository's plan; `deny.toml` cannot emit one |
| `actions/attest-build-provenance` | `v4.2.2` | fixes the predicate to SLSA build provenance, which is what `gh attestation verify` enforces by default. v4 is a thin wrapper over `actions/attest`; the wrapper is used because the predicate type then cannot be got wrong |

### 5.4 When a step fails

- **`cargo about generate` "failed to satisfy license requirements"** — a
  dependency declares a licence that is not in `docs/about.toml`'s `accepted`
  list. Add it there *and* to `deny.toml`'s `[licenses] allow`, or the two
  policies will disagree; then re-run. This is the release-side twin of the
  cargo-deny gate that already runs on every push.
- **`cargo cyclonedx` fails on a licence expression** — the tool warns rather
  than fails by default; a hard failure means a malformed expression in a
  dependency. Run it locally with `-v` to see which crate.
- **`gh release download` fails in `rebuild-and-compare`** — the release did not
  get created (look at the build job) or the tag has no release yet.
- **The attestation step fails** — nothing is published, because attestation
  runs *before* the upload. Check that the repository is public and has
  artifact attestations available (they are on all current GitHub plans for
  public repositories); a private repository would need Enterprise Cloud.
