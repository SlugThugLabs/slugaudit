-- Syntax errors and parser diagnostics across every parsed file, read from
-- the Diagnostic payload's `severity` and `message` keys.
SELECT
    f.path,
    f.language,
    e.start_line,
    json_extract(e.payload, '$.severity') AS severity,
    json_extract(e.payload, '$.message') AS message
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind = 'Diagnostic'
ORDER BY f.path, e.start_line;
