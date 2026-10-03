-- Find unhandled panics, unwraps, and expects across the codebase.
-- Useful for AI code reviews assessing crash safety and resilience.
SELECT 
    f.path,
    e.start_line,
    e.start_column,
    e.payload
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind = 'Call'
  AND (
    e.payload LIKE '%unwrap%'
    OR e.payload LIKE '%expect%'
    OR e.payload LIKE '%panic!%'
  )
ORDER BY f.path, e.start_line;
