use std::cmp::Ordering;

#[derive(Default, Debug, Clone, PartialEq)]
pub struct Match {
    pub entry: String,
    pub match_indices: Vec<usize>,
}

fn find_first_match(
    query_chars: &[char],
    entry_chars: &[char],
) -> Option<Vec<usize>> {
    let mut found: Vec<usize> = vec![];
    let mut query_idx = 0;
    let mut entry_idx = 0;
    let query_len = query_chars.len();

    while query_idx < query_len {
        let qc = query_chars[query_idx];
        if let Some(off) =
            entry_chars[entry_idx..].iter().position(|&c| c == qc)
        {
            entry_idx += off;
            found.push(entry_idx);
            entry_idx += 1;
        } else {
            return None;
        }
        query_idx += 1;
    }

    if found.len() == entry_chars.len() {
        None
    } else {
        Some(found)
    }
}

// Given the entry and the query (vectors of chars), searches the entry
// backwards from the end of the query and from the entry index given as a
// parameter, returning the match found as a .
// It's supposed there must be a match of the query in the entry backwards
// from the index given.
fn tighten_span(
    entry_chars: &[char],
    query_chars: &[char],
    entry_idx: usize,
) -> Vec<usize> {
    let query_len = query_chars.len();
    let entry_len = entry_chars.len();

    assert!(entry_len > 0);
    assert!(query_len > 0);
    assert!(entry_idx < entry_len && entry_idx >= query_len - 1);

    let mut candidate: Vec<usize> = vec![];
    let mut query_idx = query_len - 1;

    for entry_i in (0..=entry_idx).rev() {
        if entry_chars[entry_i] != query_chars[query_idx] {
            continue;
        }
        candidate.push(entry_i);
        if query_idx == 0 {
            break;
        }
        query_idx -= 1;
    }

    candidate.reverse();
    candidate
}

// Right now, the best match is the one with less span between the first and
// last chars from the query in the entry.
// It's supposed to be a match at this point.
fn find_best_match(
    entry_chars: &[char],
    query_chars: &[char],
    first_match_last_entry_idx: usize,
    mut current_min_span: usize,
) -> Vec<usize> {
    let query_len = query_chars.len();
    let entry_len = entry_chars.len();

    assert!(entry_len > 0);
    assert!(query_len > 0);
    assert!(
        first_match_last_entry_idx < entry_len
            && first_match_last_entry_idx >= query_len - 1
    );

    let mut candidate: Vec<usize> = vec![];
    let mut best: Vec<usize> = vec![];

    // We'll tight the span of the first found match.
    // The first condition from the next loop will always be true because we
    // are setting the query index as the last saved entry index.
    let mut entry_idx = first_match_last_entry_idx;
    let mut query_idx = query_len - 1;

    while entry_idx < entry_len {
        if entry_chars[entry_idx] == query_chars[query_idx] {
            candidate = tighten_span(&entry_chars, &query_chars, entry_idx);

            entry_idx = candidate[query_len - 1];
            query_idx = query_len - 1;

            let span = candidate[query_len - 1] - candidate[0];
            if span <= current_min_span {
                best = std::mem::take(&mut candidate);
                current_min_span = span;
            }
        }
        entry_idx += 1;
    }

    best
}

fn find_indices(entry: &str, query: &str) -> Option<Match> {
    if query == "" {
        return Some(Match {
            entry: entry.to_string(),
            match_indices: vec![],
        });
    }

    let query_chars: Vec<char> = query.chars().collect();
    let entry_chars: Vec<char> = entry.chars().collect();

    let first_match: Vec<usize>;
    if let Some(m) = find_first_match(&query_chars, &entry_chars) {
        first_match = m;
    } else {
        return None;
    }

    let first_match_len = first_match.len();
    let best = find_best_match(
        &entry_chars,
        &query_chars,
        first_match[first_match_len - 1], // last entry index from first match
        first_match[first_match_len - 1] - first_match[0], // current span
    );
    let byte_offset: Vec<_> = entry.char_indices().map(|(i, _)| i).collect();
    let final_indices: Vec<_> = best.iter().map(|i| byte_offset[*i]).collect();

    Some(Match {
        entry: entry.to_string(),
        match_indices: final_indices,
    })
}

// Character index of the char starting at `byte` (byte offsets in
// `match_indices` must stay byte-based for highlighting).
fn char_index(entry: &str, byte: usize) -> usize {
    entry[..byte].chars().count()
}

// Span of the selected match in characters, for relevance ranking.
fn match_span(m: &Match) -> usize {
    let first = m.match_indices.first().copied().unwrap_or(0);
    let last = m.match_indices.last().copied().unwrap_or(0);
    char_index(&m.entry, last) - char_index(&m.entry, first)
}

// Byte offset where the basename (last `/`-delimited component) starts.
fn basename_start(entry: &str) -> usize {
    entry.rfind('/').map(|i| i + 1).unwrap_or(0)
}

// Whether the selected span is fully contained in the basename.
fn span_in_basename(m: &Match) -> bool {
    let first = m.match_indices.first().copied().unwrap_or(0);
    first >= basename_start(&m.entry)
}

fn rank_cmp(a: &Match, b: &Match) -> Ordering {
    match_span(a)
        .cmp(&match_span(b))
        .then_with(|| span_in_basename(b).cmp(&span_in_basename(a)))
        .then_with(|| a.entry.chars().count().cmp(&b.entry.chars().count()))
        .then_with(|| a.entry.to_lowercase().cmp(&b.entry.to_lowercase()))
        .then_with(|| a.entry.cmp(&b.entry))
}

pub fn search(entries: &[String], query: &str) -> Vec<Match> {
    let mut matches: Vec<Match> = entries
        .iter()
        .filter_map(|e| find_indices(e, query))
        .collect();
    if !query.is_empty() {
        matches.sort_by(rank_cmp);
    }
    matches
}

#[cfg(test)]
mod tests {
    use crate::fuzzy; // Bring code from parent module into scope

    #[test]
    fn indices() {
        let e = "hello/zero";
        let mut q = "z";
        let res = fuzzy::find_indices(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6], m.match_indices);

        q = "zea";
        let res = fuzzy::find_indices(e, q);
        assert_eq!(None, res);

        let e = "a...ab";
        let res = fuzzy::find_indices(e, "ab");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![4, 5], m.match_indices);

        let e = "a......a.b....c.";
        let res = fuzzy::find_indices(e, "abc");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![7, 9, 14], m.match_indices);

        let e = "aaeaaeaeaeeaezerobeee";
        let res = fuzzy::find_indices(e, "eee");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 19, 20], m.match_indices);

        let e = "aaeaaeaeaeeaezerobee";
        let res = fuzzy::find_indices(e, "aa");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![3, 4], m.match_indices);

        let e = "aaeaaeaeaeeaezerobee";
        let res = fuzzy::find_indices(e, "eee");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![9, 10, 12], m.match_indices);

        let e = "hello/zero";
        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6, 7], m.match_indices);

        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6, 7], m.match_indices);

        let e = "/user/vieites/opt/zerobrew";
        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 19], m.match_indices);

        let res = fuzzy::find_indices(e, "e");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![24], m.match_indices);

        q = "ee";
        let res = fuzzy::find_indices(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![8, 11], m.match_indices);

        let e = "/user/vieites/opt/zerobrew";
        let q = "zer";
        let res = fuzzy::find_indices(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 19, 20], m.match_indices);
    }

    #[test]
    fn indices_multibyte() {
        // `é` is 2 bytes, so char position and byte offset diverge from
        // here on: char index of 'z' is 6, byte offset is 7.
        let e = "héllo/zero";
        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![7, 8], m.match_indices);
        assert_eq!("ze", &e[m.match_indices[0]..=m.match_indices[1]]);

        let e = "héllo";
        let res = fuzzy::find_indices(e, "e");
        assert_eq!(None, res);
    }

    fn search_entries(entries: &[&str], query: &str) -> Vec<String> {
        let owned: Vec<String> =
            entries.iter().map(|e| e.to_string()).collect();
        fuzzy::search(&owned, query)
            .iter()
            .map(|m| m.entry.clone())
            .collect()
    }

    #[test]
    fn rank_smaller_span_wins() {
        let res = search_entries(&["/x/azb", "/y/ab"], "ab");
        assert_eq!(vec!["/y/ab", "/x/azb"], res);
    }

    #[test]
    fn rank_basename_containment_beats_elsewhere_at_equal_span() {
        // All three matches have span 2: "/x/azb" in the basename,
        // "/azb/x" in the parent, "/x/a/b" across the boundary.
        let res = search_entries(&["/x/a/b", "/azb/x", "/x/azb"], "ab");
        assert_eq!(vec!["/x/azb", "/azb/x", "/x/a/b"], res);
    }

    #[test]
    fn rank_path_length_counts_chars_not_bytes() {
        // Equal byte length (6), but "/é/ab" has fewer chars (5 vs 6).
        let res = search_entries(&["/xx/ab", "/é/ab"], "ab");
        assert_eq!(vec!["/é/ab", "/xx/ab"], res);
    }

    #[test]
    fn rank_case_insensitive_then_exact_case() {
        let res = search_entries(
            &["/a/xab", "/Zebra/xab", "/apple/yab", "/A/xab"],
            "ab",
        );
        assert_eq!(vec!["/A/xab", "/a/xab", "/apple/yab", "/Zebra/xab"], res);
    }

    #[test]
    fn rank_span_uses_chars_not_bytes() {
        // "aéb" spans 2 chars but 3 bytes; ranking by byte span would
        // tie with "/azzb" and push it behind the basename-contained
        // match.
        let res = search_entries(&["/azzb", "/aéb/x"], "ab");
        assert_eq!(vec!["/aéb/x", "/azzb"], res);

        // Same disagreement with the multibyte char in the basename;
        // a byte span would tie and the shorter path would win.
        let res = search_entries(&["/azzb", "/xxxxx/aéb"], "ab");
        assert_eq!(vec!["/xxxxx/aéb", "/azzb"], res);
    }

    #[test]
    fn empty_query_preserves_candidate_order() {
        let res = search_entries(&["/zeta", "/alpha", "/mid"], "");
        assert_eq!(vec!["/zeta", "/alpha", "/mid"], res);
    }

    #[test]
    fn matching_stays_case_sensitive() {
        let res = search_entries(&["/x/ab", "/x/AB", "/x/cd"], "ab");
        assert_eq!(vec!["/x/ab"], res);
    }
}
