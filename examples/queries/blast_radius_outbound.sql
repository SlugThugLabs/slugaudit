-- Outbound dependency_edges per file, as arithmetic: for every file with an
-- import, how many edges leave it, split by resolution_kind (Resolved,
-- Unresolved, External). Counts of edges that exist in the index: no ranking,
-- no threshold, no "risky file" label. What a count means for a change you
-- are about to make is your call, not the index's.
--
-- One statement: run it as one `query` call.
SELECT
    f.path,
    e.resolution_kind,
    count(*) AS edge_count
FROM dependency_edges e
JOIN files f ON e.from_file_id = f.id
GROUP BY f.path, e.resolution_kind
ORDER BY f.path, e.resolution_kind;
