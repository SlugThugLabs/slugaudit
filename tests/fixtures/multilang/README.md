# Multilang acceptance fixture

A small, realistic polyglot repository used by the Phase 12 acceptance
contract (`tests/fixture_contract.rs`). It is checked in on purpose and is
never modified by tests — tests copy it to a temp directory before
publishing against it.

Contents:

- **Rust** crate (`rust/`) with a real `crate::util` module import
- **Python** package (`python/package/`) with relative imports, a circular
  import pair (`circular_a.py` ⇄ `circular_b.py`), and one intentionally
  malformed file (`broken.py`)
- **TypeScript** modules (`typescript/`) with a relative import and an
  external `fs` import
- **JavaScript** modules (`javascript/`) — the one language *outside* the
  old Python eight-language set — with a circular import pair
  (`circular_a.js` ⇄ `circular_b.js`)
- **Go** package (`go/`) with stdlib imports only (no project-local import
  resolution modeled for Go)
- **Ruby** scripts (`ruby/`) with `require_relative` and `require`
- Configuration (`config/`), documentation (`docs/`), scripts
  (`scripts/`, `Makefile`), and a binary file (`assets/logo.bin`)

See `CONTRACT.md` for the versioned evidence contract, and
`MANIFEST.json` for the golden manifest asserted by the test.
