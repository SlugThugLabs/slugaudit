-- Map all functions, methods, and calls handling authentication or tokens.
-- Arms the AI agent with the entire security surface in under 2ms.
SELECT 
    f.path,
    e.start_line,
    e.kind,
    e.payload
FROM evidence e
JOIN files f ON e.file_id = f.id
WHERE e.kind IN ('Symbol', 'Call')
  AND (
    LOWER(e.payload) LIKE '%auth%'
    OR LOWER(e.payload) LIKE '%token%'
    OR LOWER(e.payload) LIKE '%session%'
    OR LOWER(e.payload) LIKE '%secret%'
    OR LOWER(e.payload) LIKE '%password%'
  )
ORDER BY f.path, e.start_line;
