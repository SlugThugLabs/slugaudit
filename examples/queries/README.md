# 📖 SlugAudit Query Cookbook

Pre-built SQL and Tree-sitter query recipes for AI coding agents and human engineers.

SlugAudit indexes repository symbols, imports, calls, AST shapes, and file metadata into an ephemeral SQLite index (`.planning/slugaudit/project.db`). These queries show how an agent reads codebase facts from the index instead of reading files from disk.

---

## ⚡ Recipes

| Recipe | Target Tool | Purpose |
| :--- | :---: | :--- |
| **[`auth_attack_surface.sql`](auth_attack_surface.sql)** | `query` | Functions, methods and calls whose name mentions authentication, tokens, sessions, secrets or passwords (names read with `json_extract`). |
| **[`find_unhandled_panics.sql`](find_unhandled_panics.sql)** | `query` | Call sites whose callee is `unwrap`, `expect` or `panic`, matched on `callee_name`. |
| **[`syntax_diagnostics.sql`](syntax_diagnostics.sql)** | `query` | Syntax errors and parser diagnostics per file, with their `severity` and `message`. |
| **[`rust_functions.scm`](rust_functions.scm)** | `structure` | Match exact Rust function declarations, parameters, return types, and bodies. |
| **[`blast_radius_outbound.sql`](blast_radius_outbound.sql)** | `query` | Per-file outbound `dependency_edges` counts by `resolution_kind`. Counts only — no ranking. |
| **[`blast_radius_inbound.sql`](blast_radius_inbound.sql)** | `query` | Per-file inbound `dependency_edges` counts and distinct importing files. Counts only — no ranking. |
| **[`blast_radius_closure.sql`](blast_radius_closure.sql)** | `query` | The transitive import closure of one file in both directions (`imports`, `imported_by`) over resolved edges. |
| **[`evidence_payload_keys.md`](evidence_payload_keys.md)** | reference | Every `evidence.payload` key, its JSON type, and the `evidence.kind` it belongs to. Read this instead of probing payload shapes by trial. |

Each `.sql` file is **one statement**: the `query` tool runs one statement per
call, so a recipe runs as-is (edit only the path a recipe names). Read
`evidence.payload` with `json_extract` on the key you want (see
`evidence_payload_keys.md`), never with `LIKE` over the payload text: that also
matches docs, annotations and caller names.

---

## Paging

`query`, `leads`, `structure` and `finding_read` return the same `page` object
beside their rows:

| Field | Meaning |
| :--- | :--- |
| `returned` | Items in this page. |
| `total` | Items the whole request selects (exact; `null` when the tool cannot count it: `structure` always, `query` when counting ran out of budget). |
| `complete` | `true` when nothing remains to page. |
| `next_offset` | Always present; `null` when nothing remains, otherwise pass it as `offset` on the next call. |
| `page_bytes` | The byte budget that applied, after the clamp to the server ceiling. |
| `bytes_used` | Serialized JSON bytes of this page's items. |
| `limited_by` | What cut the page: `none`, `limit`, `page_bytes`, `ceiling`, `match_cap`, `scan_window` or `time_budget`. |
| `revision_id` | The index revision the page was read from. |

Pages are cut by bytes, not by a row count; a recipe that may return many
rows pages by passing `offset` = `page.next_offset` and `expected_revision` =
`page.revision_id`, so a changed index is refused (`stale_revision`) instead of
skipping or repeating rows. `structure` alone has a second cursor beside
`page`: when `complete` is `false` and `next_offset` is `null`, repeat with
`start_after_file` = `next_start_after_file` and `offset` 0.

---

## 🛠️ How AI Agents Use These Recipes

When asking your AI agent (Claude Code, Cursor, Hermes, Codex, etc.) to audit a codebase, it can execute these queries directly over MCP:

### Example: Running SQL Evidence Queries
```json
{
  "name": "query",
  "arguments": {
    "sql": "SELECT f.path, e.start_line, json_extract(e.payload, '$.name') AS name FROM evidence e JOIN files f ON e.file_id = f.id WHERE e.kind = 'Symbol' AND lower(json_extract(e.payload, '$.name')) LIKE '%login%'"
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
