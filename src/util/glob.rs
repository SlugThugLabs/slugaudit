//! Linear-time path glob matching shared by Cargo workspace filtering and
//! SQLite `slugaudit_path_match(path, pattern)` queries.

/// Matches `candidate` against a `/`-separated glob `pattern` where:
/// - `**` as a full path segment matches zero or more whole segments,
/// - `*` within a segment matches zero or more non-`/` characters, and
/// - `?` within a segment matches any single non-`/` character.
#[must_use]
pub(crate) fn path_glob_matches(pattern: &str, candidate: &str) -> bool {
    if pattern == candidate {
        return true;
    }
    let pat_segs: Vec<&str> = pattern.split('/').collect();
    let cand_segs: Vec<&str> = candidate.split('/').collect();
    // KEEP saturating: cand_segs comes from splitting an in-memory &str, well below usize::MAX
    let cols = cand_segs.len().saturating_add(1);
    let mut prev = vec![false; cols];
    let mut curr = vec![false; cols];
    if let Some(first) = prev.get_mut(0) {
        *first = true;
    }

    for &pat in &pat_segs {
        curr.fill(false);
        if pat == "**" {
            let mut matched = false;
            for j in 0..cols {
                if prev.get(j) == Some(&true) {
                    matched = true;
                }
                if let Some(slot) = curr.get_mut(j) {
                    *slot = matched;
                }
            }
        } else {
            for (j, &cand) in cand_segs.iter().enumerate() {
                if prev.get(j) == Some(&true) && segment_glob_matches(pat, cand) {
                    // KEEP saturating: j < cand_segs.len(), well below usize::MAX
                    let next_col = j.saturating_add(1);
                    if let Some(slot) = curr.get_mut(next_col) {
                        *slot = true;
                    }
                }
            }
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev.last() == Some(&true)
}

/// Matches a tool `pattern` (`leads`, `structure`, `mechanic`) against a stored
/// project-relative `path`:
/// - Without `*` or `?`, matches by substring (`path.contains(pattern)`).
/// - With `*` or `?` and no `/`, `*` matches across `/` (`*auth*`, `*.rs`).
/// - With `*` or `?` and `/`, uses segment-aware [`path_glob_matches`].
#[must_use]
pub(crate) fn search_path_matches(candidate: &str, pattern: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return candidate.contains(pattern);
    }
    if !pattern.contains('/') {
        return segment_glob_matches(pattern, candidate);
    }
    path_glob_matches(pattern.trim_start_matches('/'), candidate)
}

fn segment_glob_matches(pattern: &str, text: &str) -> bool {
    let (mut p, mut t) = (0_usize, 0_usize);
    let mut star: Option<(usize, usize)> = None;

    while let Some(t_ch) = text.get(t..).and_then(|rest| rest.chars().next()) {
        let p_ch = pattern.get(p..).and_then(|rest| rest.chars().next());
        match p_ch {
            Some('*') => {
                // KEEP saturating: p + 1 <= pattern.len(), well below usize::MAX
                p = p.saturating_add(1);
                star = Some((p, t));
            }
            Some('?') => {
                // KEEP saturating: p + 1 <= pattern.len(), well below usize::MAX
                p = p.saturating_add(1);
                // KEEP saturating: t + t_ch.len_utf8() <= text.len(), well below usize::MAX
                t = t.saturating_add(t_ch.len_utf8());
            }
            Some(ch) if ch == t_ch => {
                // KEEP saturating: p + ch.len_utf8() <= pattern.len(), well below usize::MAX
                p = p.saturating_add(ch.len_utf8());
                // KEEP saturating: t + t_ch.len_utf8() <= text.len(), well below usize::MAX
                t = t.saturating_add(t_ch.len_utf8());
            }
            _ => {
                let Some((star_p, star_t)) = star else {
                    return false;
                };
                let Some(star_ch) = text.get(star_t..).and_then(|rest| rest.chars().next()) else {
                    return false;
                };
                // KEEP saturating: star_t + star_ch.len_utf8() <= text.len(), well below usize::MAX
                let next_t = star_t.saturating_add(star_ch.len_utf8());
                star = Some((star_p, next_t));
                p = star_p;
                t = next_t;
            }
        }
    }

    while pattern.get(p..).is_some_and(|tail| tail.starts_with('*')) {
        // KEEP saturating: p + 1 <= pattern.len(), well below usize::MAX
        p = p.saturating_add(1);
    }
    p == pattern.len()
}
