#[derive(Default, Debug, Clone, PartialEq)]
pub struct Match {
    pub entry: String,
    pub match_indexes: Vec<usize>,
}

fn find_indexes(entry: &str, query: &str) -> Option<Match> {
    if query == "" {
        return Some(Match {
            entry: entry.to_string(),
            match_indexes: vec![],
        });
    } else if query.len() == 1 {
        if let Some(f) = entry.rfind(query) {
            return Some(Match {
                entry: entry.to_string(),
                match_indexes: vec![f],
            });
        } else {
            return None;
        }
    }

    // Forward: looking for the end.

    // `end`: the index in `entry` that matches the last char in `query`
    let mut end: Option<usize> = None;
    let mut entry_idx = 0;
    let mut last_qc: char = '\0';
    let mut qcs = query.chars();
    let mut ecs = entry.chars();

    loop {
        if let Some(c) = qcs.next() {
            last_qc = c;
            end = None;
            while let Some(ec) = ecs.next() {
                // z
                if c == ec {
                    end = Some(entry_idx);
                    entry_idx += 1;
                    break;
                }
                entry_idx += 1;
            }
        } else {
            while let Some(ec) = ecs.next() {
                if last_qc == ec {
                    end = Some(entry_idx);
                }
                entry_idx += 1;
            }
            break;
        }
    }

    let final_end;
    if let Some(e) = end {
        final_end = e;
    } else {
        return None;
    }

    // Backwards: setting the matches.

    // Char index -> byte offset, since `indexes` must be byte offsets
    // (consumers slice `entry` with them), but the scan below walks chars.
    let byte_offsets: Vec<usize> =
        entry.char_indices().map(|(b, _)| b).collect();

    let mut indexes: Vec<usize> = vec![];
    let mut r = (0..=final_end).rev();
    let mut qcsr = query.chars().rev();
    let ecs_vec: Vec<char> = entry.chars().collect();

    while let Some(c) = qcsr.next() {
        while let Some(i) = r.next() {
            if c == ecs_vec[i] {
                indexes.push(byte_offsets[i]);
                break;
            }
        }
    }

    indexes.reverse();

    return Some(Match {
        entry: entry.to_string(),
        match_indexes: indexes,
    });
}

pub fn search(entries: &[&str], query: &str) -> Vec<Match> {
    entries
        .iter()
        .filter_map(|e| find_indexes(e, query))
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::fuzzy; // Bring code from parent module into scope

    #[test]
    fn indexes() {
        let e = "hello/zero";
        let mut q = "z";
        let res = fuzzy::find_indexes(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6], m.match_indexes);

        q = "ze";
        let res = fuzzy::find_indexes(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![6, 7], m.match_indexes);

        let e = "/user/vieites/opt/zerobrew";
        let res = fuzzy::find_indexes(e, q);
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![18, 24], m.match_indexes);
    }

    #[test]
    fn indexes_multibyte() {
        // `é` is 2 bytes, so char position and byte offset diverge from
        // here on: char index of 'z' is 6, byte offset is 7.
        let e = "héllo/zero";
        let res = fuzzy::find_indexes(e, "ze");
        assert_ne!(None, res);
        let m = res.unwrap();
        assert_eq!(vec![7, 8], m.match_indexes);
        assert_eq!("ze", &e[m.match_indexes[0]..=m.match_indexes[1]]);
    }
}
