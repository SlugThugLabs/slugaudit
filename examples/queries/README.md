# 📖 SlugAudit Query Cookbook

Pre-built SQL and Tree-sitter query recipes for AI coding agents and human engineers.

SlugAudit indexes repository symbols, imports, calls, AST shapes, and file metadata into an ephemeral SQLite index (`.planning/slugaudit/project.db`). These queries demonstrate how AI agents can extract codebase facts in sub-milliseconds without reading files from disk.

---

## ⚡ Recipes

| Recipe | Target Tool | Purpose |
| :--- | :---: | :--- |
| **[`auth_attack_surface.sql`](auth_attack_surface.sql)** | `query` | Map every function, method, and call touching authentication, tokens, or sessions. |
| **[`find_unhandled_panics.sql`](find_unhandled_panics.sql)** | `query` | Find crash-prone `unwrap()`, `expect()`, and `panic!()` invocations across all files. |
| **[`syntax_diagnostics.sql`](syntax_diagnostics.sql)** | `query` | Pull syntax errors and parser diagnostics discovered during tree-sitter indexing. |
| **[`rust_functions.scm`](rust_functions.scm)** | `structure` | Match exact Rust function declarations, parameters, return types, and bodies. |
| **[`blast_radius.sql`](blast_radius.sql)** | `query` | Per-file inbound/outbound `dependency_edges` counts, plus the transitive closure in either direction. Counts only — no ranking. |
| **[`evidence_payload_keys.md`](evidence_payload_keys.md)** | reference | Every `evidence.payload` key, its JSON type, and the `evidence.kind` it belongs to. Read this instead of probing payload shapes by trial. |

Each `.sql` file holds one or more statements; the `query` tool runs **one
statement per call**, so paste them one at a time (the comments name the
statement boundaries).

---

## 🛠️ How AI Agents Use These Recipes

When asking your AI agent (Claude Code, Cursor, Hermes, Codex, etc.) to audit a codebase, it can execute these queries directly over MCP:

### Example: Running SQL Evidence Queries
```json
{
  "name": "query",
  "arguments": {
    "sql": "SELECT f.path, e.start_line, e.payload FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND LOWER(e.payload) LIKE '%login%';"
  }
}
```

### Example: Running Tree-sitter AST Structure Queries (Single File)
```json
{
  "name": "structure",
  "arguments": {
    "file": "src/main.rs",
    "query": "(function_item name: (identifier) @name body: (block) @body)"
  }
}
```

### Example: Running Multi-File Tree-sitter AST Structure Queries
```json
{
  "name": "structure",
  "arguments": {
    "language": "rust",
    "pattern": "src/**/*.rs",
    "query": "(function_item name: (identifier) @name)"
  }
}
```
