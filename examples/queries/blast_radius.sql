-- dependency_edges, as arithmetic, per file. Counts of edges that exist in
-- the index: no ranking, no threshold, no "risky file" label. What any count
-- means for a change you are about to make is your call, not the index's.
--
-- Each statement below is ONE `query` call (the query tool runs one read-only
-- statement per call). Replace `src/main.rs` with the path you care about. An
-- empty closure is an answer, not a broken query: no indexed file imports (or
-- is imported by) that path along resolved edges.

-- 1. Per-file edge counts, both directions. `outbound` is every edge whose
--    from_file_id is this file, split by resolution_kind; `inbound` is every
--    edge that lands on it. Only edges with a to_file_id land on a project
--    file, so inbound rows are always Resolved.
SELECT
    f.path,
    'outbound' AS direction,
    e.resolution_kind,
    count(*) AS edge_count
FROM dependency_edges e
JOIN files f ON e.from_file_id = f.id
GROUP BY f.path, e.resolution_kind

UNION ALL

SELECT
    f.path,
    'inbound' AS direction,
    e.resolution_kind,
    count(*) AS edge_count
FROM dependency_edges e
JOIN files f ON e.to_file_id = f.id
GROUP BY f.path, e.resolution_kind

ORDER BY f.path, direction, resolution_kind;

-- 2. Transitive closure outbound: every project file `src/main.rs` imports,
--    directly or through other project files. UNION (not UNION ALL) is what
--    makes an import cycle terminate.
WITH RECURSIVE reachable(file_id) AS (
    SELECT id FROM files WHERE path = 'src/main.rs'
    UNION
    SELECT e.to_file_id
    FROM dependency_edges e
    JOIN reachable r ON e.from_file_id = r.file_id
    WHERE e.to_file_id IS NOT NULL
)
SELECT f.path, f.language
FROM reachable r
JOIN files f ON f.id = r.file_id
WHERE f.path <> 'src/main.rs'
ORDER BY f.path;

-- 3. Transitive closure inbound: every project file that imports
--    `src/main.rs`, directly or through other project files — the same
--    traversal with the edges reversed.
WITH RECURSIVE importers(file_id) AS (
    SELECT id FROM files WHERE path = 'src/main.rs'
    UNION
    SELECT e.from_file_id
    FROM dependency_edges e
    JOIN importers i ON e.to_file_id = i.file_id
)
SELECT f.path, f.language
FROM importers i
JOIN files f ON f.id = i.file_id
WHERE f.path <> 'src/main.rs'
ORDER BY f.path;
