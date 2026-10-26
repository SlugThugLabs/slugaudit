-- The transitive closure of one file along resolved dependency_edges, in both
-- directions: every project file it imports directly or through other project
-- files (`imports`), and every project file that imports it the same way
-- (`imported_by`). UNION (not UNION ALL) inside each recursive CTE is what
-- makes an import cycle terminate. An empty result is an answer, not a broken
-- query: no indexed file is connected to that path along resolved edges.
--
-- One statement: run it as one `query` call. Replace `src/main.rs` (it
-- appears once, in `target`) with the path you care about.
WITH RECURSIVE
    target(file_id) AS (
        SELECT id FROM files WHERE path = 'src/main.rs'
    ),
    reachable(file_id) AS (
        SELECT file_id FROM target
        UNION
        SELECT e.to_file_id
        FROM dependency_edges e
        JOIN reachable r ON e.from_file_id = r.file_id
        WHERE e.to_file_id IS NOT NULL
    ),
    importers(file_id) AS (
        SELECT file_id FROM target
        UNION
        SELECT e.from_file_id
        FROM dependency_edges e
        JOIN importers i ON e.to_file_id = i.file_id
    )
SELECT 'imports' AS direction, f.path, f.language
FROM reachable r
JOIN files f ON f.id = r.file_id
WHERE r.file_id NOT IN (SELECT file_id FROM target)
UNION ALL
SELECT 'imported_by' AS direction, f.path, f.language
FROM importers i
JOIN files f ON f.id = i.file_id
WHERE i.file_id NOT IN (SELECT file_id FROM target)
ORDER BY direction, path;
