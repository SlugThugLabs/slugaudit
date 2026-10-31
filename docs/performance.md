# Performance

SlugAudit hashes every file on every call (see [how it works](how-it-works.md)). This page says what that costs.

These numbers are the cost of the freshness check. They are not the cost of the query itself. A `query` or `leads` answer from the database is separate and usually small by comparison.

## Measured per-file cost

Test machine: AMD Ryzen 9 5900X, 32 GB DDR4-2133, ZFS on NVMe. Warm file cache. One thread. Each file: stat, open, read, BLAKE3.

| File size | Time per file |
|---|---|
| 4 KB or less | about 11 µs |
| 16 KB | about 15 µs |
| 64 KB | about 54 µs |
| 256 KB | about 134 µs |
| 1 MB | about 0.5 ms |
| 4 MB | about 1.9 ms |
| 32 MB | about 20 ms |

A larger real-world test: 122,249 files, 1.96 GB of real crate source.

| Run | Time |
|---|---|
| One thread, warm cache | 3.5 s |
| First pass (partly cold cache) | 16.4 s |

## Rough guide per call

One thread, warm cache, nothing changed:

| Project size | Files | Time per call |
|---|---|---|
| Small | 2,000 files, 30 MB | about 45 ms |
| Medium | 10,000 files, 160 MB | about 0.23 s |
| Large | about 40,000 files, 350 MB | about 0.7 s |
| Linux kernel size | about 82,000 files, 1.4 GB | about 1.9 s |

These are estimates from the per-file numbers. Your machine will differ.

## Single thread or many?

The reading and hashing of files runs on a pool of worker threads. The pool size is the number of files or the number of CPU cores, whichever is smaller, and at most 8. So on a machine with 8 or more cores, hashing is spread over 8 threads. The numbers above are single-thread numbers. A multi-core run should be faster on a warm cache. That has not been measured.

Not everything in a call is parallel. The directory walk and the check of the stored text against its hash each run on one thread. They add to the figures above. So read the tables as the cost of the read-and-hash step, not as a guarantee for the whole call.

## What changes the cost

- **Cold disk.** The first call after a reboot, or after the files fell out of the cache, reads from disk. It can be much slower, as the 16.4 s first pass shows.
- **Not enough RAM to keep the project cached.** If the machine cannot hold the project's files in the OS cache, every call pays disk speed. This is the main thing that makes SlugAudit slow on a small machine.
- **Memory use.** During a call, the text of the files read is held in memory. The total is capped by `max_total_import_bytes`. See [configuration](configuration.md).
- **Changed files.** Files that changed are re-parsed and rewritten. A large first import costs far more than a call where nothing changed. The first call also downloads grammars for languages it has not seen (see [languages](languages.md)).
- **Big files.** Cost grows with file size. Files above `max_file_bytes` are skipped without being read in full (the first 8,000 bytes are still read to tell binary from text). Files above 2 MiB are read and hashed but not parsed.

## Limits that bound the time

| Limit | `lean` | `audit` and `adaptive` |
|---|---|---|
| Whole sync (`max_sync_wall_clock`) | 120 s | 1200 s |
| One query (`max_query_wall_clock`) | 10 s | 60 s |
| One `structure` search (`max_structure_execution_time`) | 10 s | 60 s |
| One file's parse (`max_parse_wall_clock_ms`) | 5 s | 5 s |

A sync that runs over its limit fails with `sync_budget_exhausted`. See [troubleshooting](troubleshooting.md).
