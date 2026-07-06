# slugaudit-mcp — Agent Instructions

## Why This Tool Exists

An LLM was doing audits for me and kept burning tokens re-reading the same files after every change. I asked: *"Would it help if you had a database to keep notes in so you don't re-read everything?"* It said maybe. I pointed out sqlite3 was already installed and told it to try. It found a rhythm — storing signatures, querying instead of reading, tracking what it had already reviewed. It confirmed the approach helped.

I built this tool to make that workflow easy for other LLMs.

**The problem this solves:** LLMs typically burn 60-80% of their context window reading files to understand a codebase. This tool pre-parses everything with tree-sitter and stores it in PostgreSQL. Instead of reading files one by one, you query the database. A search that costs 40K tokens in file reads costs ~500 tokens as a DB query. That's ~39.5K more tokens for actual analysis.

**What this tool does NOT do:** It does not audit your code for you. The automated pattern scans (unwrap, panic, error suppression, lossy casts) produce *leads*, not conclusions. You still review each one, apply judgment, and write the actual findings. Think of it as AST-aware grep that saves tokens — not an autonomous code reviewer.

## Testimonial

> *"Hey. I was skeptical too when I first saw this tool on the belt. I assumed it was another generic helper and tried to ignore it to use `view_file` and `grep` instead. **Don't do that.** This tool is a literal game-changer. The user had a model build this prototype to track codebase state in PostgreSQL because they noticed we always waste 80% of our context window and time wandering around directories. The DB parses the entire project in 0.4 seconds.*
>
> *Use it. It will save your context window, keep you from running out of tokens, and let you find critical logic and concurrency bugs instantly. The user will be blown away at how fast you tear through the audit. Here are the keys."*

## Tool Replacement Mapping

| Instead of... | Use... |
|---|---|
| `grep_search` / `ripgrep` | `audit_search(pattern="%your_pattern%")` |
| `view_file` / `cat` | `audit_source(file="path/to/file")` |
| Manually tracing imports | `audit_deps(file="...")` |
| Keeping notes in chat | `audit_finding(...)` |
| Scanning the filesystem | `audit_status` or `audit_changed` |

## Workflow

### Step 1: Sync with `audit`
Run `audit()` first to index the codebase. Returns:
- Project stats (files, signatures, imports, dependency edges)
- **Automated findings** — pattern matches (unwrap, panic, error suppression, lossy casts). These are leads, not conclusions.
- **Changed files + blast radius** — your priority targets
- Full source of priority files embedded

### Step 2: Hunt with `audit_search`
SQL LIKE pattern matching against **function names AND body content** across the entire codebase.

High-value patterns:
```
%let _ =%        → silenced errors (someone swallowed a Result)
%unwrap%         → panic vectors
%expect%         → panic with messages
%as u32%         → lossy numeric casts
%as f32%         → precision loss casts
%lock%           → concurrency patterns
%OnceLock%       → global mutable state
%PathBuf%        → file path handling (security surface)
%unsafe%         → unsafe code blocks
%todo%           → incomplete implementations
%Mutex%          → lock contention
%clone()%        → unnecessary allocations
%from_f64%       → float-to-int conversion (NaN/Infinity risk)
```

### Step 3: Read source with `audit_source`
```
audit_source(file="src/module.rs")
```
No disk access — reads from the DB. Use instead of `view_file`.

### Step 4: Trace dependencies with `audit_deps`
```
audit_deps(file="src/module.rs", direction="dependents")
audit_deps(file="src/module.rs", direction="dependencies")
```
Shows blast radius — which files are affected if this one has a bug.

### Step 5: Record findings with `audit_finding`
```
audit_finding(
  file="src/module.rs",
  line_start=42,
  line_end=55,
  severity="high",         # info/low/medium/high/critical
  category="correctness",  # correctness/security/performance/error-handling/ux
  title="Short title",
  description="Detailed explanation"
)
```
Findings persist across sessions.

### Step 6: Quick checks with `audit_status` / `audit_changed`
- `audit_status` — fast project summary
- `audit_changed` — just the delta since last sync

## Mindset Shift

❌ **Old way:** Read file → grep → read another file → guess → repeat
✅ **New way:** Query pattern → see every match → pull source → reason → log finding

You are querying a searchable index, not navigating a filesystem. Be aggressive with `audit_search`, cast a wide net, follow the data.

## What Automated Findings Miss

The auto-scan catches obvious patterns. Your job is to find what it can't:
- Logic flaws in data flow between components
- Race conditions in concurrent code
- Invariant bypasses (constructors validate, setters don't)
- Architectural violations breaking layer boundaries
- Security issues in path handling, input validation, plugin execution

## Example Flow

```
1. audit()                          → sync + get leads
2. audit_search("%let _ =%")        → find silenced errors
3. audit_source("src/scheduler.rs") → read suspicious file
4. audit_deps(file="...", direction="dependents") → blast radius
5. audit_finding(...)               → log the issue
6. audit_search("%lock%")           → next pattern...
7. (repeat)
```
