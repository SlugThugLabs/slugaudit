# Plan Audit — Phase 6

Status: audited — FAIL

## Verdict

The plan correctly avoids pretending to be a compiler, but its relationship
resolution policy remains too vague to prevent false edges. A dependency graph
that is confidently wrong is worse for an AI than an explicitly unresolved
edge.

## Findings

1. No universal import-to-file resolution algorithm exists across 306
   languages. The plan needs language-pack confidence and resolution-method
   fields.
2. Relative path, package, workspace, generated, and external imports are not
   separated in the candidate model.
3. Ambiguous candidates are mentioned but no deterministic output contract or
   user-visible ambiguity record is defined.
4. Rebuilding affected edges does not define the affected set or complexity.
5. A deleted target, renamed target, and unresolved target have different
   historical behavior but are not distinguished.
6. Circular imports are tested for termination but not deterministic graph
   output or bounded traversal.
7. The plan does not define whether import strings are stored verbatim for AI
   inspection; they must be.
8. No graph revision/edge provenance contract is defined.
9. No maximum incoming/outgoing result budget is defined.
10. No incremental versus full rebuild threshold is defined.

## Required corrections

Define `ResolutionKind`, confidence, candidate list, ambiguity, provenance,
revision, and raw import retention. Add bounded deterministic graph queries and
an explicit “unresolved is not external” rule.

## Testing / logging

Test every resolution class, ambiguity, cycles, deletion/rename, generated
files, large fan-in/fan-out, and deterministic ordering. Log resolution counts,
ambiguities, unresolved imports, and duration without source secrets.
