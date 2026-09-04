# Functional style vs for loops

Learned the heuristic for when to use iterator combinators (`map`, `filter_map`, `flat_map`, `find`) vs imperative `for` loops in Rust: functional style for pure transformations where each element maps independently; `for` loops for complex branching, side effects, early exit, or when fighting the borrow checker in closures.

Applied directly to `src/fuzzy/mod.rs`: the `map` + `is_some()` + `unwrap()` pattern is a textbook case for `filter_map`. The `is_match` boolean field is derivable from `match_indices.is_empty()`; keeping it would be premature extraction.

**Evidence**: User showed a concrete example from `contx`, asked for more comparative cases, and can now evaluate which tool fits.

**Implications**: Can refactor `src/fuzzy/mod.rs` search function. Next: apply the heuristic to other iterator-heavy spots in `contx`.