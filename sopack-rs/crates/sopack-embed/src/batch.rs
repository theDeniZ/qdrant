//! Length-sorted, token-budget batching (SOPACK-1.0-PLAN.md §3.3): sort
//! texts by descending token length, then greedily fill each batch with as
//! many as fit a padded-token `budget`. Sorting first means every batch
//! pads to close to its own longest member instead of the corpus's
//! longest, which the M0 spike measured as vector-neutral (cosine ≥
//! 0.9999, in fact 1.0000000 at every budget tried) — this module is
//! purely about *which indices* land in which batch; the engine restores
//! original order by scattering each batch's outputs back by index.

/// Plans batches over `lens` (one token length per text, same order the
/// caller's texts are in) under a padded-token `budget`. Returns each
/// batch as the list of *original* indices it contains, in the packed
/// (length-descending) order — the caller embeds `lens[i]`-ordered text
/// `i` for each `i` in a batch, then scatters results back by `i`.
///
/// A `budget` smaller than the longest text still yields a batch of size 1
/// for it (never drops a text, never truncates the batch to zero).
pub fn plan_batches(lens: &[usize], budget: usize) -> Vec<Vec<usize>> {
    let mut order: Vec<usize> = (0..lens.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(lens[i]));
    let mut batches = Vec::new();
    let mut i = 0;
    while i < order.len() {
        let longest = lens[order[i]].max(1);
        let n = (budget / longest).max(1).min(order.len() - i);
        batches.push(order[i..i + n].to_vec());
        i += n;
    }
    batches
}

/// Scatters `batches` of `(original_index, value)` pairs — produced by
/// embedding each `plan_batches` batch in turn — back into original-text
/// order. Panics (a programmer error, not a runtime one) if a batch plan
/// and its results disagree on which indices were covered, since a silent
/// gap there would mean a missing or duplicated vector in the pack.
pub fn scatter<T>(n: usize, batches: Vec<(Vec<usize>, Vec<T>)>) -> Vec<T> {
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for (indices, values) in batches {
        assert_eq!(
            indices.len(),
            values.len(),
            "a batch's index list and result list must be the same length"
        );
        for (idx, value) in indices.into_iter().zip(values) {
            let slot = out
                .get_mut(idx)
                .unwrap_or_else(|| panic!("batch index {idx} out of range for {n} texts"));
            assert!(slot.is_none(), "batch index {idx} was embedded twice");
            *slot = Some(value);
        }
    }
    out.into_iter()
        .enumerate()
        .map(|(i, v)| v.unwrap_or_else(|| panic!("index {i} was never embedded by any batch")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn covers_every_index_exactly_once(batches: &[Vec<usize>], n: usize) {
        let mut seen = HashSet::new();
        for b in batches {
            for &i in b {
                assert!(seen.insert(i), "index {i} appeared in more than one batch");
            }
        }
        assert_eq!(seen.len(), n, "every index 0..{n} must appear exactly once");
        for i in 0..n {
            assert!(seen.contains(&i));
        }
    }

    #[test]
    fn every_index_appears_exactly_once() {
        let lens = vec![10, 500, 3, 200, 512, 1, 50, 400];
        let batches = plan_batches(&lens, 512);
        covers_every_index_exactly_once(&batches, lens.len());
    }

    #[test]
    fn each_batch_respects_the_padded_budget() {
        let lens = vec![100, 100, 100, 100, 100, 5];
        let budget = 350;
        let batches = plan_batches(&lens, budget);
        for b in &batches {
            let longest = b.iter().map(|&i| lens[i]).max().unwrap();
            let padded = longest * b.len();
            assert!(
                padded <= budget,
                "batch {b:?} pads to {padded} > budget {budget}"
            );
        }
    }

    #[test]
    fn a_single_text_longer_than_the_budget_still_gets_its_own_batch() {
        let lens = vec![10_000];
        let batches = plan_batches(&lens, 512);
        assert_eq!(batches, vec![vec![0]]);
    }

    #[test]
    fn batches_are_sorted_longest_first_within_the_plan() {
        let lens = vec![5, 500, 50, 5000];
        let batches = plan_batches(&lens, 5000);
        // index 3 (len 5000) must be planned before index 1 (len 500),
        // which must be planned before the two short ones.
        let flat: Vec<usize> = batches.into_iter().flatten().collect();
        let pos = |i: usize| flat.iter().position(|&x| x == i).unwrap();
        assert!(pos(3) < pos(1));
        assert!(pos(1) < pos(0) || pos(1) < pos(2));
    }

    #[test]
    fn empty_input_yields_no_batches() {
        assert_eq!(plan_batches(&[], 512), Vec::<Vec<usize>>::new());
    }

    #[test]
    fn scatter_restores_original_order() {
        let lens = vec![30, 10, 20, 5];
        let batches = plan_batches(&lens, 60);
        let results: Vec<(Vec<usize>, Vec<String>)> = batches
            .into_iter()
            .map(|b| {
                let vals = b.iter().map(|&i| format!("text-{i}")).collect();
                (b, vals)
            })
            .collect();
        let out = scatter(lens.len(), results);
        assert_eq!(out, vec!["text-0", "text-1", "text-2", "text-3"]);
    }

    #[test]
    #[should_panic(expected = "was embedded twice")]
    fn scatter_panics_on_duplicate_index() {
        scatter(2, vec![(vec![0, 1], vec!["a", "b"]), (vec![0], vec!["c"])]);
    }

    #[test]
    #[should_panic(expected = "never embedded")]
    fn scatter_panics_on_missing_index() {
        scatter(3, vec![(vec![0, 1], vec!["a", "b"])]);
    }
}
