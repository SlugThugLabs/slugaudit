-- Find syntax errors and parser diagnostics across all indexed files.
-- Lets AI auditors spot broken syntax without compiling or running external linters.
SELECT 
    f.path,
    f.language,
    e.start_line,
    e.payload AS diagnostic_message
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind = 'Diagnostic'
ORDER BY f.path, e.start_line;
