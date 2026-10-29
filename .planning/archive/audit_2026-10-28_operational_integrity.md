# Operational Integrity & Runtime Fidelity Audit — SlugAudit
Date 2026-10-28. Source /home/slugaudit @ 4477552 (clean tree). Binary smoke-tested as built at that commit. Read-only; nothing edited, committed or rebuilt. Method: clerk inventory -> runtime smoke -> five gate auditors (sonnet) -> eight Wave 2 verifiers on PLAUSIBLE items. Findings are by code trace plus smoke captures; no finding was reproduced by a new run unless it says "smoke".

## Verdicts
- GATE 1 Clean Terminal & Honest Diagnostics: FAIL (5 findings; 1 CRITICAL masking)
- GATE 2 Ergonomics (CLI/backend): FAIL (5 findings)
- GATE 3 Latency & Event Economy: FAIL (4 findings)
- GATE 4 Data Loss & Silent Discard: FAIL (6 findings; 3 data-loss)
- GATE 5 Band-Aid & Environment Workaround: FAIL (7 findings)

## Findings, ranked
Format: path:line | symptom | why | minimal fix | status.

### CRITICAL
C1 (G1) src/sync/manager_health_support.rs:135-146; src/sync/publish_log.rs:30-45; src/sync/manager_meta.rs:219-232; src/sync/sweep_reconcile.rs:58-66; src/sync/sample_batch.rs:329-336; src/watch/errors.rs:30
- Symptom: a failed sync/publish logs `WARN publish failed error_kind=sample_read path_id=<64-hex blake3>`. No OS cause (EACCES/ENOENT/ENOSPC), no file name, no SQLite message. The operator cannot recover either. The MCP client gets the full text (`publishing a new revision: {error}`), the terminal does not. map_publish_error carries the full error but logs it at debug only.
- Why: sync-layer log sites drop the error Display and log a static category plus redacted_path (src/util.rs:32-41). Path hashed to protect a local operator who already owns the machine.
- Fix: log `error = %source` and the real path at warn, once (connection_guards.rs:259 already logs raw paths). Hashing, if kept, opt-in.
- Status: CONFIRMED (code trace; smoke b_debug.stderr for the sibling WARN).

C2 (G4) src/sync/manager_orchestration.rs:254-267 -> src/sync/manager_meta.rs:41-54
- Symptom: restarting the server, or a second session on the same project, silently deletes every finding recorded by other sessions. Response says nothing.
- Why: first sync per process runs `DELETE FROM findings WHERE session_id != current`; the once-per-process guard does not protect against a second process sharing the DB.
- Fix: mark prior-session findings `stale` (status exists) or purge only provably dead sessions; report deleted count.
- Status: CONFIRMED (trace, not run).

C3 (G4) src/project/activation.rs:246 (from src/tools/project_control.rs:327)
- Symptom: `project_control off` runs remove_dir_all(.planning/slugaudit): DB, findings, config.json (user profile/mechanic overrides), config.lock and any other user files in that directory, no confirmation. project_control.rs:54 comment and tool docs do not mention config.json.
- Fix: delete only DB, sidecars and lock; keep config.json; or require explicit confirm and say so in the response.
- Status: CONFIRMED.

C4 (G4) src/sync/manager_orchestration.rs:142-169; src/tools/project_control.rs:296-301
- Symptom: schema/contract bump or corruption false-positive wipes the DB file and sidecars, which also holds agent-authored findings that cannot be re-derived. Only a log line; tool response silent.
- Fix: export/re-import findings across discard, or report `findings_discarded: N`.
- Status: CONFIRMED (conflicts with the "database stays disposable" non-negotiable because findings live in it).

### HIGH
H1 (G3) src/sync/manager_health.rs:173 via manager_orchestration.rs:102 and src/sweep.rs:97
- Symptom: every tool call on a healthy watcher walks and stats the whole tree first. Fixed cost per call scales with repo size: over 16 ms on small trees, likely seconds at 100k+ files (estimate, not measured); hard budget 120 s (src/model/profile.rs:149). The watcher exists to avoid this.
- Fix: sweep only when not Healthy or when an interval has elapsed; otherwise trust the dirty set.
- Status: CONFIRMED path; magnitude estimated.

H2 (G1/G2/G5) src/main.rs:133
- Symptom: clean startup writes INFO to stderr, including rmcp's client peer_info dump (rmcp service.rs:1367) and handler/server.rs:585; our own INFO/WARN per call at src/server_runner.rs:130,194,203, so every failed tool call WARNs. Smoke b_unset.stderr.
- Why: `EnvFilter::new("info")` applies to all targets including dependencies.
- Fix: default `warn` (or `warn,slugaudit_mcp_rust=info`).
- Status: CONFIRMED (smoke).

H3 (G1/G2) src/server_runner.rs:160-166, :194-203
- Symptom: comment says ErrorData::message is never logged; rmcp logs `response error ... message` (paths, SQL fragments) at WARN anyway (b_debug.stderr). Our own line shows code="none", no cause. Two log lines per failure, plaintext only from the dependency; redaction contract is false.
- Fix: one policy: log error.message once in log_completion and cap rmcp at error, or accept redaction and cap rmcp; fix the comment either way.
- Status: CONFIRMED (smoke).

H4 (G1) src/project/config.rs:121-130 (load_or_default)
- Symptom: malformed or unreadable project config is replaced with Self::default(); only a warn log with a hashed root_id and cause without path. No tool response field reports it. Operator runs with different limits than their file says and is not told.
- Fix: return Result; surface in tool response / fail the call.
- Status: CONFIRMED (Wave 2 V1: callers tools/mod.rs:194, reconcile/options.rs:32, publish_cas.rs:55 carry no status).

H5 (G5) src/model/profile.rs:47-70; src/model/limits.rs:365-390; src/main.rs:127-140
- Symptom: a typo in SLUGAUDIT_* or RUST_LOG is warned/ignored and the server runs with other limits (RUST_LOG falls back to info). Checks do not fail closed.
- Fix: exit non-zero on a set-but-invalid variable.
- Status: CONFIRMED.

H6 (G5) src/model/system_ram.rs:41-50
- Symptom: RAM detection failure invents an 8 GiB machine and proceeds (warn only; reported as real in tools/health.rs:463). Inconsistent with src/store/netfs.rs:95, which fails closed when /proc/self/mountinfo is unreadable, so a /proc-less container cannot open the project at all while RAM silently guesses.
- Fix: Result; require explicit profile when undetectable; use statfs/sysconf instead of parsing /proc.
- Status: CONFIRMED (Wave 2 V8).

H7 (G2/G5) src/main.rs:53-57, :62-67, :84
- Symptom: SIGTERM: no handler, process dies by signal, no drain or log (smoke d_term.txt). EOF before initialize exits 1 and prints Rust Debug `Error: ConnectionClosed("initialize request")` (smoke e_eofonly.err). All main-returned errors print Debug form.
- Fix: tokio::signal select with the serve future; treat pre-initialize EOF as clean exit with one line; route errors through cli::err + exit(1).
- Status: CONFIRMED. Note: leaked result-store files claim downgraded (see Dismissed).

H8 (G2) src/main.rs:35,53,42-56
- Symptom: exit codes only 0/1; usage errors and runtime failures both 1; bad flag reads `unknown command "--bogus"` (smoke a_badflag). gate_common/args.rs already uses 2.
- Fix: exit 2 on parse errors.
- Status: CONFIRMED.

H9 (G4) src/install/path.rs:27
- Symptom: install rewrites ~/.bashrc/.zshrc/.profile with non-atomic std::fs::write; crash/full disk leaves a truncated shell rc.
- Fix: append with OpenOptions::append, or temp+fsync+rename preserving mode (pattern exists at config_store.rs replace_atomically).
- Status: CONFIRMED.

### MEDIUM
M1 (G2/V2) two error channels: bad args -> result isError:true; nonexistent path -> JSON-RPC -32602. Cause is rmcp 3.5.1: router/tool.rs:146-155 into_tool_argument_error turns INVALID_PARAMS with the prefix "failed to deserialize parameters:" (handler/server/tool.rs:243) into CallToolResult::error; our ErrorData (e.g. manager_orchestration.rs:329-335) lacks the prefix. Our src never emits isError:false with an error body (grep of is_error/isError/CallToolResult in non-test src: none). Fix: map argument-parse failures to one channel or document both. CONFIRMED.
M2 (G3/V3) src/server_runner.rs:27,138-152; src/store/connection_guards.rs:19,290; src/sync/revision.rs:139: no per-project single-flight; concurrent calls only share an 8-permit semaphore. A call needing a write lock waits up to BUSY_TIMEOUT 5 s behind another call's Immediate publish transaction and can fail Busy. Reader calls are not blocked (WAL). Fix: per-project single-flight sync. PARTIAL: Busy-reporting path not traced.
M3 (G3) src/store/connection_guards.rs:305-320: std::thread::sleep(20 ms) WAL retry up to 5 s while holding a blocking-pool permit; first open of a fresh DB only. Fix: fail fast Busy. CONFIRMED, narrow.
M4 (G3/V4) src/server_runner.rs:311-312, :320-321, :344-345: one tokio::spawn per progress event; Started/Completed unthrottled (Sampling throttled 100 ms, :336-340, :365-367). Pile-up under a blocked transport is suspected, not proven (rmcp transport not read). Fix: one bounded channel + drain task. PARTIAL.
M5 (G3/G5) RUST_LOG=debug emits 145 KB stderr per short session (smoke); stderr writer is blocking so an undrained host stalls the server. Fix: drop payload logging at debug; non-blocking appender. Smoke-CONFIRMED volume; stall inferred.
M6 (G4/V5) src/install.rs:122-123 retain_previous_binary: plain fs::copy onto slugaudit.prev (truncates the existing good copy in place, no fsync); comment at :111-112 ("complete file rather than caught mid-write") is false. Fix: temp+fsync+rename. CONFIRMED.
M7 (G5) install.sh:91-95: cargo-install fallback skips retain_previous, installs to ~/.cargo/bin but banner says ${TARGET_PATH}; install.sh:16-20,35-36,86: no trap, temp dir leaks on curl failure; `echo -e`/emoji assume bash+UTF-8. CONFIRMED (leak), PLAUSIBLE (rest).
M8 (G5) src/update_fetch.rs:86,111: shells out to curl from PATH, partial -o file can remain on failure, ASSET hardcoded linux/x86_64; :56 `unwrap_or("")` accepts malformed asset entries. Fix: in-process HTTPS client, remove partial dest, error on bad entry. CONFIRMED.
M9 (G5) src/install/path.rs:38-60: unset/unknown $SHELL -> ~/.bashrc. Fix: refuse and print the line. CONFIRMED.
M10 (G2) src/util/style.rs:27: colour gated on is_terminal only; NO_COLOR and TERM=dumb ignored -> raw escapes in menu on a dumb TTY. CONFIRMED by code.
M11 (G2) src/cli.rs:64-69: `-v` rejected while `-V` works; connect/disconnect silently ignore extra args. CONFIRMED.
M12 (G4/V6) src/connect/config_file.rs:157,173: client configs replaced with hard-coded mode 0600 regardless of existing mode; re-serialised (:240, :260) with keys re-sorted (no serde_json preserve_order, Cargo.toml:76). Commented files fail parse and the write is refused, so comments are not lost. By the same Cargo.toml fact, G4's claim that config.json key order is preserved (config_store.rs:303-309) is likely wrong; not separately verified. Also no trailing newline on config.json. CONFIRMED (0600, ordering).
M13 (G4) src/project/gitignore.rs:171-205: content preserved byte for byte, but parent dir not fsynced after rename (config_store.rs:372 does) and read-modify-write unlocked. CONFIRMED, minor.
M14 (G1) src/main.rs:164-190: invalid RUST_LOG falls back to info after a warn (merged with H5).
Known prior defects (PRAGMA poisoning, symlink writes, 0-based lines, runtime .so download, 50 MB file) were not re-audited.

## Clean-startup transcript summary (smoke, out/smoke/SUMMARY.md)
- --help/-h/--version: exit 0, stdout only. Bad flag: `unknown command "--bogus"`, exit 1. Bad subcommand likewise.
- MCP stdio in a fresh project: stdout contained only JSON-RPC (b_*.nonjson empty per summary); first response ~4 ms (c_timing.txt: 3.8-4.1). No writes to project dir (proj_before/after).
- Stderr on a CLEAN startup is NOT empty: INFO from rmcp (peer_info dump, handler/server.rs:585) because of src/main.rs:133. Every failed tool call adds a WARN; rmcp adds its own `response error` WARN.
- RUST_LOG=debug: 145 KB stderr. SIGTERM: killed by signal. EOF pre-initialize: exit 1.

## Dismissed (checked, not a finding)
- src/prompt.rs:38,50-57 confirm declines on non-TTY and reports read failures: no hang, answer is "no".
- src/menu.rs:22,58 EOF on stdin exits the menu: no spin.
- src/main.rs:42,51 broken pipe exits 0: correct for pipelines. src/main.rs:23-27 lossy argv avoids a panic.
- src/cli/usage.rs:25,50,53 -h/--help/--version exit 0, clean stdout.
- src/util.rs:52 canonical_or_self and :117 lock_or_recover log the cause then recover; probe.rs:12,31 and install.rs:200-210 log errors.
- src/prompt.rs:32, store/netfs.rs:300: unused-binding under cfg(test)/unsupported target, no Result dropped.
- `.ok()?` parse/prefix returns None as documented non-match: sync/discovery.rs:189,204, reconcile/pipeline.rs:113, watch/path.rs:22, result_store/owner.rs:110, cursor.rs:76,85, ignore_rules.rs:119. sync/sample.rs:118 lossy UTF-8 is flagged in output.
- connect_exec.rs:165 unwrap_or_else picks a message and keeps the source.
- watch/manager/events.rs:70 notify callback uses try_lock, defers, cap 4096, overflow -> Desynced: no heavy work on raw events.
- watch/manager/scopes.rs:97-113 and scope_apply.rs:66-76 lock scope short and rare.
- server_runner.rs:148 all tool work via spawn_blocking; :337 throttle guard dropped before spawn.
- publish_cas.rs:81-108 retry bounded (MAX_CAS_RETRIES + one deadline); reconcile/barrier.rs:63 checks deadline each pass; store/connection.rs:173 one-time busy_timeout.
- config_store.rs:285-298 poll: bounded, returns typed Locked/Busy, SQLite-quirk workaround; kept.
- Result store: result_file.rs:34-56 files unlinked at creation; kill window cleaned by startup sweep (owner.rs:114-160). A stale <session>.lock survives SIGTERM/SIGKILL (store.rs:255-264 Drop skipped) and is swept next start (owner.rs:178-183). Wave 2 V7 downgraded G5's "file leak" to this; TROUBLESHOOTING.md:406-419 matches. Eviction (registry.rs:132-179) is reported as result_expired; mechanic_scan_cache eviction counted in health.rs:191.
- config_cleanup.rs:158 removes only pattern-matched tmp files; config_store.rs:337-353 temp+fsync+rename+dir fsync; install_place.rs:25-88 atomic rename, failures logged; install.rs:10-38 SLUGTHUG_* overrides documented.
- parse/fetch_gate.rs:19 10-minute cooldown is visible; registry_types.rs:180, result_store/store.rs:78 refuse unsupported OS explicitly.
- registry_types.rs:217-220 APPDATA fallback: Windows, unsupported elsewhere, likely dead code (low; unverified).
- src/bin/check_* dev gates: not shipped.

## Not verified
- Magnitude of H1 on large trees (no timing run); M4 stall under blocked transport; M2 Busy-to-caller mapping; M12 config.json key-order claim; Windows paths. No run was made beyond Wave 0 smoke.
