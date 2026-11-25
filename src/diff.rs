//! Minimal list update: replacing a whole GtkStringList of ~1000 items costs
//! ~25 ms, so only the changed middle part is spliced.

/// For turning `old` into `new` with one splice: returns `(position,
/// n_removed, added)` where `added` is the range of `new` to insert.
pub fn splice<T: PartialEq>(old: &[T], new: &[T]) -> (usize, usize, std::ops::Range<usize>) {
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let max_suffix = old.len().min(new.len()) - prefix;
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    (
        prefix,
        old.len() - prefix - suffix,
        prefix..new.len() - suffix,
    )
}

#[cfg(test)]
mod tests {
    use super::splice;

    fn apply(old: &[i32], new: &[i32]) -> Vec<i32> {
        let (pos, removed, added) = splice(old, new);
        let mut v = old.to_vec();
        v.splice(pos..pos + removed, new[added].iter().copied());
        v
    }

    #[test]
    fn unchanged_is_a_no_op() {
        assert_eq!(splice(&[1, 2, 3], &[1, 2, 3]), (3, 0, 3..3));
    }

    #[test]
    fn prepend_one() {
        assert_eq!(splice(&[1, 2, 3], &[9, 1, 2, 3]), (0, 0, 0..1));
    }

    #[test]
    fn recopy_moves_item_to_top() {
        // cliphist dedups: re-copying entry 3 moves it to the top.
        let (old, new) = ([1, 2, 3, 4, 5], [3, 1, 2, 4, 5]);
        assert_eq!(splice(&old, &new), (0, 3, 0..3));
        assert_eq!(apply(&old, &new), new);
    }

    #[test]
    fn filtering_and_back() {
        for (old, new) in [
            (vec![1, 2, 3, 4], vec![2, 4]),
            (vec![2, 4], vec![1, 2, 3, 4]),
            (vec![], vec![1, 2]),
            (vec![1, 2], vec![]),
            (vec![1, 1, 1], vec![1, 1]),
        ] {
            assert_eq!(apply(&old, &new), new, "{old:?} -> {new:?}");
        }
    }
}
