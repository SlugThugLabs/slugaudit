-- Inbound dependency_edges per file, as arithmetic: for every project file
-- another project file imports, how many edges land on it and from how many
-- distinct importing files. Only an edge with a to_file_id lands on a project
-- file, so every inbound edge is Resolved. Counts only: no ranking, no
-- threshold, no "risky file" label.
--
-- One statement: run it as one `query` call.
SELECT
    f.path,
    count(*) AS edge_count,
    count(DISTINCT e.from_file_id) AS importing_files
FROM dependency_edges e
JOIN files f ON e.to_file_id = f.id
GROUP BY f.path
ORDER BY f.path;
