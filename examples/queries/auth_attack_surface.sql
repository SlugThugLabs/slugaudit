-- Functions, methods and calls whose name mentions authentication, tokens,
-- sessions, secrets or passwords. Names only: the symbol's `name` and the
-- call's `callee_text` are read with json_extract, never by LIKE over the
-- whole payload text, which would also match docs, annotations and the
-- caller's own name. A name match is a place to look, not a finding.
SELECT path, start_line, kind, name
FROM (
    SELECT
        f.path,
        e.start_line,
        e.kind,
        COALESCE(
            json_extract(e.payload, '$.name'),
            json_extract(e.payload, '$.callee_text')
        ) AS name
    FROM evidence e
    JOIN files f ON e.file_id = f.id
    WHERE e.kind IN ('Symbol', 'Call')
)
WHERE lower(name) LIKE '%auth%'
   OR lower(name) LIKE '%token%'
   OR lower(name) LIKE '%session%'
   OR lower(name) LIKE '%secret%'
   OR lower(name) LIKE '%password%'
ORDER BY path, start_line;
