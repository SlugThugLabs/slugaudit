# Architecture

This fixture exists to exercise SlugAudit's evidence contract.

- A Rust crate with a module import (`rust/`)
- A Python package with relative imports and a circular pair
  (`python/package/`)
- TypeScript and JavaScript modules (`typescript/`, `javascript/`)
- Go and Ruby packages (`go/`, `ruby/`)
- One intentionally malformed Python file (`python/package/broken.py`)
- One binary file (`assets/logo.bin`)
