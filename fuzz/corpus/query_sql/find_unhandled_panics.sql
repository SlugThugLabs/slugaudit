-- Find unhandled panics, unwraps, and expects across the codebase.
-- Useful for AI code reviews assessing crash safety and resilience.
-- Match the callee name, never the payload text: a `LIKE '%expect%'` over
-- the whole payload also matches a caller merely named `handle_expect`,
-- which is not an expect call site.
SELECT
    f.path,
    e.start_line,
    e.start_column,
    e.payload
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind = 'Call'
  AND json_extract(e.payload, '$.callee_name') IN ('unwrap', 'expect', 'panic')
ORDER BY f.path, e.start_line;
