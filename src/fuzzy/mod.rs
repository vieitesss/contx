#[derive(Default, Debug, Clone, PartialEq)]
pub struct Match {
    pub entry: String,
    pub match_indexes: Vec<usize>,
}

fn find_indices(entry: &str, query: &str) -> Option<Match> {
    if query == "" {
        return Some(Match {
            entry: entry.to_string(),
            match_indexes: vec![],
        });
    }

    let mut indices: Vec<usize> = vec![];
    let query_chars: Vec<_> = query.chars().collect();
    let entry_chars: Vec<_> = entry.chars().collect();
    let entry_len = entry_chars.len();
    let query_len = query_chars.len();
    let mut entry_idx = 0;
    let mut query_idx = 0;

    // Looking for the full match.

    while query_idx < query_len {
        let qc = query_chars[query_idx];
        if let Some(off) =
            entry_chars[entry_idx..].iter().position(|&c| c == qc)
        {
            entry_idx += off;
            indices.push(entry_idx);
            entry_idx += 1;
        } else {
            return None;
        }
        query_idx += 1;
    }

    let indexes_len = indices.len();

    if indexes_len == 0 || query_len != indexes_len {
        return None;
    }

    assert!(entry_idx == indices[indexes_len - 1] + 1);
    assert!(query_idx == query_len);

    let mut min_span = indices[indexes_len - 1] - indices[0];
    let mut candidate: Vec<usize> = vec![];

    // We'll tight the span of the first found match.
    // The first condition from the next loop will always be true because we
    // are setting the query index with the last saved entry index.
    entry_idx = indices[indexes_len - 1];
    query_idx = query_len - 1;

    println!("entry={entry};query={query}");
    println!("indices={:?}", indices);
    while entry_idx < entry_len {
        if entry_chars[entry_idx] == query_chars[query_idx] {
            println!("query_idx={query_idx:?}");
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

            println!("candidate={candidate:?}");
            let span = candidate[0] - candidate[query_len - 1];
            entry_idx = candidate[0];
            query_idx = query_len - 1;

            if span <= min_span {
                candidate.reverse();
                indices = std::mem::take(&mut candidate);
                min_span = span;
            }
            candidate.clear();
        }

        entry_idx += 1;
    }

    let byte_offset: Vec<_> = entry.char_indices().map(|(i, _)| i).collect();

    let final_indices: Vec<_> =
        indices.iter().map(|i| byte_offset[*i]).collect();

    Some(Match {
        entry: entry.to_string(),
        match_indexes: final_indices,
    })
}

pub fn search(entries: &[&str], query: &str) -> Vec<Match> {
    entries
        .iter()
        .filter_map(|e| find_indices(e, query))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::fuzzy; // Bring code from parent module into scope

    #[test]
    fn indexes() {
        let e = "hello/zero";
        let mut q = "z";
        let res = fuzzy::find_indices(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6], m.match_indexes);

        q = "zea";
        let res = fuzzy::find_indices(e, q);
        assert_eq!(None, res);

        let e = "a...ab";
        let res = fuzzy::find_indices(e, "ab");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![4, 5], m.match_indexes);

        let e = "a......a.b....c.";
        let res = fuzzy::find_indices(e, "abc");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![7, 9, 14], m.match_indexes);

        let e = "aaeaaeaeaeeaezerobeee";
        let res = fuzzy::find_indices(e, "eee");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 19, 20], m.match_indexes);

        let e = "aaeaaeaeaeeaezerobee";
        let res = fuzzy::find_indices(e, "eee");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![9, 10, 12], m.match_indexes);

        let e = "hello/zero";
        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6, 7], m.match_indexes);

        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6, 7], m.match_indexes);

        let e = "/user/vieites/opt/zerobrew";
        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 19], m.match_indexes);

        let res = fuzzy::find_indices(e, "e");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![24], m.match_indexes);

        q = "ee";
        let res = fuzzy::find_indices(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![8, 11], m.match_indexes);

        let e = "/user/vieites/opt/zerobrew";
        let q = "zer";
        let res = fuzzy::find_indices(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 19, 20], m.match_indexes);
    }

    #[test]
    fn indexes_multibyte() {
        // `é` is 2 bytes, so char position and byte offset diverge from
        // here on: char index of 'z' is 6, byte offset is 7.
        let e = "héllo/zero";
        let res = fuzzy::find_indices(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![7, 8], m.match_indexes);
        assert_eq!("ze", &e[m.match_indexes[0]..=m.match_indexes[1]]);

        let e = "héllo";
        let res = fuzzy::find_indices(e, "e");
        assert_eq!(None, res);
    }
}
