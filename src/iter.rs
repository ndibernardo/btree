use std::borrow::Borrow;
use std::cmp::Ordering;
use std::fmt;
use std::iter::FusedIterator;
use std::ops::Bound;
use std::ops::RangeBounds;

use crate::node::BranchNode;
use crate::node::LeafNode;
use crate::node::Node;
use crate::node::SearchSlot;

type Item<'a, K, V> = (&'a K, &'a V);

/// Error returned when range bounds cannot describe an ordered interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// The start bound sorts after the end bound.
    StartAfterEnd,
    /// Equal bounds exclude their only possible shared point.
    EmptyExcludedBounds,
}

impl fmt::Display for RangeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartAfterEnd => formatter.write_str("range start is after range end"),
            Self::EmptyExcludedBounds => {
                formatter.write_str("equal range bounds must both be included")
            }
        }
    }
}

impl std::error::Error for RangeError {}

/// Immutable iterator over a tree's entries in key order.
pub struct Iter<'a, K, V, const CAPACITY: usize> {
    front: Cursor<'a, K, V, CAPACITY>,
    back: Cursor<'a, K, V, CAPACITY>,
    remaining: usize,
}

impl<'a, K, V, const CAPACITY: usize> Iter<'a, K, V, CAPACITY> {
    pub(crate) fn new(root: &'a Node<K, V, CAPACITY>, length: usize) -> Self {
        Self {
            front: Cursor::at_first(root),
            back: Cursor::at_last(root),
            remaining: length,
        }
    }
}

impl<'a, K, V, const CAPACITY: usize> Iterator for Iter<'a, K, V, CAPACITY> {
    type Item = Item<'a, K, V>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let entry = self.front.next_entry()?;
        self.remaining -= 1;
        Some(entry)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<K, V, const CAPACITY: usize> DoubleEndedIterator for Iter<'_, K, V, CAPACITY> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.remaining == 0 {
            return None;
        }

        let entry = self.back.next_back_entry()?;
        self.remaining -= 1;
        Some(entry)
    }
}

impl<K, V, const CAPACITY: usize> ExactSizeIterator for Iter<'_, K, V, CAPACITY> {}
impl<K, V, const CAPACITY: usize> FusedIterator for Iter<'_, K, V, CAPACITY> {}

/// Immutable double-ended iterator over entries within validated key bounds.
pub struct Range<'a, K, V, const CAPACITY: usize> {
    front: Cursor<'a, K, V, CAPACITY>,
    back: Cursor<'a, K, V, CAPACITY>,
    state: TraversalState,
}

impl<'a, K: Ord, V, const CAPACITY: usize> Range<'a, K, V, CAPACITY> {
    pub(crate) fn new<Q, R>(root: &'a Node<K, V, CAPACITY>, bounds: &R) -> Result<Self, RangeError>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
        R: RangeBounds<Q>,
    {
        validate_bounds(bounds)?;

        Ok(Self {
            front: Cursor::at_start_bound(root, bounds.start_bound()),
            back: Cursor::at_end_bound(root, bounds.end_bound()),
            state: TraversalState::Active,
        })
    }

    fn finish(&mut self) {
        self.state = TraversalState::Finished;
    }
}

impl<'a, K: Ord, V, const CAPACITY: usize> Iterator for Range<'a, K, V, CAPACITY> {
    type Item = Item<'a, K, V>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.state == TraversalState::Finished {
            return None;
        }

        let back_key = match self.back.peek_back_entry() {
            Some((key, _value)) => key,
            None => {
                self.finish();
                return None;
            }
        };
        let entry = match self.front.next_entry() {
            Some(entry) => entry,
            None => {
                self.finish();
                return None;
            }
        };

        match entry.0.cmp(back_key) {
            Ordering::Less => Some(entry),
            Ordering::Equal => {
                self.finish();
                Some(entry)
            }
            Ordering::Greater => {
                self.finish();
                None
            }
        }
    }
}

impl<K: Ord, V, const CAPACITY: usize> DoubleEndedIterator for Range<'_, K, V, CAPACITY> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.state == TraversalState::Finished {
            return None;
        }

        let front_key = match self.front.peek_entry() {
            Some((key, _value)) => key,
            None => {
                self.finish();
                return None;
            }
        };
        let entry = match self.back.next_back_entry() {
            Some(entry) => entry,
            None => {
                self.finish();
                return None;
            }
        };

        match front_key.cmp(entry.0) {
            Ordering::Less => Some(entry),
            Ordering::Equal => {
                self.finish();
                Some(entry)
            }
            Ordering::Greater => {
                self.finish();
                None
            }
        }
    }
}

impl<K: Ord, V, const CAPACITY: usize> FusedIterator for Range<'_, K, V, CAPACITY> {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TraversalState {
    Active,
    Finished,
}

struct Cursor<'a, K, V, const CAPACITY: usize> {
    leaf: Option<&'a LeafNode<K, V, CAPACITY>>,
    entry_index: usize,
    ancestors: Vec<AncestorFrame<'a, K, V, CAPACITY>>,
}

impl<'a, K, V, const CAPACITY: usize> Cursor<'a, K, V, CAPACITY> {
    fn at_first(root: &'a Node<K, V, CAPACITY>) -> Self {
        let mut cursor = Self::empty();
        cursor.descend_to_first(root);
        cursor
    }

    fn at_last(root: &'a Node<K, V, CAPACITY>) -> Self {
        let mut cursor = Self::empty();
        cursor.descend_to_last(root);
        cursor
    }

    const fn empty() -> Self {
        Self {
            leaf: None,
            entry_index: 0,
            ancestors: Vec::new(),
        }
    }

    fn next_entry(&mut self) -> Option<Item<'a, K, V>> {
        loop {
            let leaf = self.leaf?;
            if let Some(entry) = leaf.entry_at(self.entry_index) {
                self.entry_index += 1;
                return Some(entry);
            }
            self.move_to_next_leaf();
        }
    }

    fn next_back_entry(&mut self) -> Option<Item<'a, K, V>> {
        loop {
            let leaf = self.leaf?;
            if self.entry_index > 0 {
                self.entry_index -= 1;
                return leaf.entry_at(self.entry_index);
            }
            self.move_to_previous_leaf();
        }
    }

    fn peek_entry(&mut self) -> Option<Item<'a, K, V>> {
        loop {
            let leaf = self.leaf?;
            if let Some(entry) = leaf.entry_at(self.entry_index) {
                return Some(entry);
            }
            self.move_to_next_leaf();
        }
    }

    fn peek_back_entry(&mut self) -> Option<Item<'a, K, V>> {
        loop {
            let leaf = self.leaf?;
            if self.entry_index > 0 {
                return leaf.entry_at(self.entry_index - 1);
            }
            self.move_to_previous_leaf();
        }
    }

    fn descend_to_first(&mut self, mut node: &'a Node<K, V, CAPACITY>) {
        loop {
            match node {
                Node::Leaf(leaf) => {
                    self.leaf = Some(leaf);
                    self.entry_index = 0;
                    return;
                }
                Node::Branch(branch) => {
                    self.ancestors.push(AncestorFrame::new(branch, 0));
                    let Some(child) = branch.child_at(0) else {
                        self.leaf = None;
                        return;
                    };
                    node = child;
                }
            }
        }
    }

    fn descend_to_last(&mut self, mut node: &'a Node<K, V, CAPACITY>) {
        loop {
            match node {
                Node::Leaf(leaf) => {
                    self.leaf = Some(leaf);
                    self.entry_index = leaf.entry_count();
                    return;
                }
                Node::Branch(branch) => {
                    let child_index = branch.child_count().saturating_sub(1);
                    self.ancestors.push(AncestorFrame::new(branch, child_index));
                    let Some(child) = branch.child_at(child_index) else {
                        self.leaf = None;
                        return;
                    };
                    node = child;
                }
            }
        }
    }

    fn move_to_next_leaf(&mut self) {
        loop {
            let next = self.ancestors.last().and_then(AncestorFrame::next_child);
            match next {
                Some((child_index, child)) => {
                    if let Some(frame) = self.ancestors.last_mut() {
                        frame.child_index = child_index;
                    }
                    self.descend_to_first(child);
                    return;
                }
                None if self.ancestors.pop().is_some() => {}
                None => {
                    self.leaf = None;
                    return;
                }
            }
        }
    }

    fn move_to_previous_leaf(&mut self) {
        loop {
            let previous = self
                .ancestors
                .last()
                .and_then(AncestorFrame::previous_child);
            match previous {
                Some((child_index, child)) => {
                    if let Some(frame) = self.ancestors.last_mut() {
                        frame.child_index = child_index;
                    }
                    self.descend_to_last(child);
                    return;
                }
                None if self.ancestors.pop().is_some() => {}
                None => {
                    self.leaf = None;
                    return;
                }
            }
        }
    }
}

impl<'a, K, V, const CAPACITY: usize> Cursor<'a, K, V, CAPACITY> {
    fn at_start_bound<Q>(root: &'a Node<K, V, CAPACITY>, bound: Bound<&Q>) -> Self
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match bound {
            Bound::Unbounded => Self::at_first(root),
            Bound::Included(key) => Self::at_key(root, key, BoundSide::StartIncluded),
            Bound::Excluded(key) => Self::at_key(root, key, BoundSide::StartExcluded),
        }
    }

    fn at_end_bound<Q>(root: &'a Node<K, V, CAPACITY>, bound: Bound<&Q>) -> Self
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match bound {
            Bound::Unbounded => Self::at_last(root),
            Bound::Included(key) => Self::at_key(root, key, BoundSide::EndIncluded),
            Bound::Excluded(key) => Self::at_key(root, key, BoundSide::EndExcluded),
        }
    }

    fn at_key<Q>(root: &'a Node<K, V, CAPACITY>, key: &Q, side: BoundSide) -> Self
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let mut cursor = Self::empty();
        let mut node = root;

        loop {
            match node {
                Node::Leaf(leaf) => {
                    cursor.leaf = Some(leaf);
                    cursor.entry_index = side.entry_index(leaf.search(key));
                    return cursor;
                }
                Node::Branch(branch) => {
                    let child_index = branch.child_index_for_key(key);
                    cursor
                        .ancestors
                        .push(AncestorFrame::new(branch, child_index));
                    let Some(child) = branch.child_at(child_index) else {
                        cursor.leaf = None;
                        return cursor;
                    };
                    node = child;
                }
            }
        }
    }
}

struct AncestorFrame<'a, K, V, const CAPACITY: usize> {
    branch: &'a BranchNode<K, V, CAPACITY>,
    child_index: usize,
}

impl<'a, K, V, const CAPACITY: usize> AncestorFrame<'a, K, V, CAPACITY> {
    const fn new(branch: &'a BranchNode<K, V, CAPACITY>, child_index: usize) -> Self {
        Self {
            branch,
            child_index,
        }
    }

    fn next_child(&self) -> Option<(usize, &'a Node<K, V, CAPACITY>)> {
        let next_index = self.child_index.saturating_add(1);
        self.branch
            .child_at(next_index)
            .map(|child| (next_index, child))
    }

    fn previous_child(&self) -> Option<(usize, &'a Node<K, V, CAPACITY>)> {
        let previous_index = self.child_index.checked_sub(1)?;
        self.branch
            .child_at(previous_index)
            .map(|child| (previous_index, child))
    }
}

#[derive(Clone, Copy)]
enum BoundSide {
    StartIncluded,
    StartExcluded,
    EndIncluded,
    EndExcluded,
}

impl BoundSide {
    const fn entry_index(self, slot: SearchSlot) -> usize {
        match (self, slot) {
            (Self::StartIncluded | Self::EndExcluded, SearchSlot::Occupied(index)) => index.get(),
            (Self::StartExcluded | Self::EndIncluded, SearchSlot::Occupied(index)) => {
                index.get().saturating_add(1)
            }
            (
                Self::StartIncluded | Self::StartExcluded | Self::EndIncluded | Self::EndExcluded,
                SearchSlot::Vacant(index),
            ) => index.get(),
        }
    }
}

fn validate_bounds<Q, R>(bounds: &R) -> Result<(), RangeError>
where
    Q: Ord + ?Sized,
    R: RangeBounds<Q>,
{
    match (bounds.start_bound(), bounds.end_bound()) {
        (Bound::Unbounded, Bound::Unbounded | Bound::Included(_) | Bound::Excluded(_))
        | (Bound::Included(_) | Bound::Excluded(_), Bound::Unbounded) => Ok(()),
        (Bound::Included(start), Bound::Included(end)) => match start.cmp(end) {
            Ordering::Less | Ordering::Equal => Ok(()),
            Ordering::Greater => Err(RangeError::StartAfterEnd),
        },
        (
            Bound::Included(start) | Bound::Excluded(start),
            Bound::Included(end) | Bound::Excluded(end),
        ) => match start.cmp(end) {
            Ordering::Less => Ok(()),
            Ordering::Equal => Err(RangeError::EmptyExcludedBounds),
            Ordering::Greater => Err(RangeError::StartAfterEnd),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::collections::BTreeMap as StandardBTreeMap;
    use std::ops::Bound;

    use proptest::collection;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;
    use proptest::test_runner::TestCaseResult;

    use super::RangeError;
    use crate::BTree;
    use crate::InsertOutcome;

    type AccountBalances = BTree<String, u64, 3>;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct GeneratedAccountId(u16);

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct GeneratedBalanceCents(u64);

    #[derive(Clone, Copy, Debug)]
    enum GeneratedBound {
        Unbounded,
        Included(GeneratedAccountId),
        Excluded(GeneratedAccountId),
    }

    impl GeneratedBound {
        const fn as_bound(self) -> Bound<GeneratedAccountId> {
            match self {
                Self::Unbounded => Bound::Unbounded,
                Self::Included(account_id) => Bound::Included(account_id),
                Self::Excluded(account_id) => Bound::Excluded(account_id),
            }
        }

        fn admits_start(self, account_id: &GeneratedAccountId) -> bool {
            match self {
                Self::Unbounded => true,
                Self::Included(bound) => account_id >= &bound,
                Self::Excluded(bound) => account_id > &bound,
            }
        }

        fn admits_end(self, account_id: &GeneratedAccountId) -> bool {
            match self {
                Self::Unbounded => true,
                Self::Included(bound) => account_id <= &bound,
                Self::Excluded(bound) => account_id < &bound,
            }
        }
    }

    fn generated_account_id() -> impl Strategy<Value = GeneratedAccountId> {
        (1_001_u16..1_129).prop_map(GeneratedAccountId)
    }

    fn generated_balance() -> impl Strategy<Value = GeneratedBalanceCents> {
        (100_u64..10_000_000).prop_map(GeneratedBalanceCents)
    }

    fn generated_bound() -> impl Strategy<Value = GeneratedBound> {
        prop_oneof![
            Just(GeneratedBound::Unbounded),
            generated_account_id().prop_map(GeneratedBound::Included),
            generated_account_id().prop_map(GeneratedBound::Excluded),
        ]
    }

    fn expected_range_error(start: GeneratedBound, end: GeneratedBound) -> Option<RangeError> {
        match (start, end) {
            (GeneratedBound::Unbounded, _) | (_, GeneratedBound::Unbounded) => None,
            (GeneratedBound::Included(start), GeneratedBound::Included(end)) => {
                match start.cmp(&end) {
                    Ordering::Less | Ordering::Equal => None,
                    Ordering::Greater => Some(RangeError::StartAfterEnd),
                }
            }
            (
                GeneratedBound::Included(start) | GeneratedBound::Excluded(start),
                GeneratedBound::Included(end) | GeneratedBound::Excluded(end),
            ) => match start.cmp(&end) {
                Ordering::Less => None,
                Ordering::Equal => Some(RangeError::EmptyExcludedBounds),
                Ordering::Greater => Some(RangeError::StartAfterEnd),
            },
        }
    }

    fn assert_ranges_match_standard<const CAPACITY: usize>(
        entries: &[(GeneratedAccountId, GeneratedBalanceCents)],
        start: GeneratedBound,
        end: GeneratedBound,
    ) -> TestCaseResult {
        let mut actual = BTree::<GeneratedAccountId, GeneratedBalanceCents, CAPACITY>::new();
        let mut standard = StandardBTreeMap::new();

        entries.iter().copied().for_each(|(account_id, balance)| {
            let _outcome = actual.insert(account_id, balance);
            standard.insert(account_id, balance);
        });

        let actual_range = actual.range((start.as_bound(), end.as_bound()));
        match (actual_range, expected_range_error(start, end)) {
            (Err(actual_error), Some(expected_error)) => {
                prop_assert_eq!(actual_error, expected_error);
            }
            (Ok(range), None) => {
                let expected_entries: Vec<_> = standard
                    .iter()
                    .filter(|(account_id, _balance)| {
                        start.admits_start(account_id) && end.admits_end(account_id)
                    })
                    .map(|(account_id, balance)| (*account_id, *balance))
                    .collect();
                let actual_entries: Vec<_> = range
                    .map(|(account_id, balance)| (*account_id, *balance))
                    .collect();
                prop_assert_eq!(&actual_entries, &expected_entries);

                let Ok(reverse_range) = actual.range((start.as_bound(), end.as_bound())) else {
                    return Err(TestCaseError::fail(
                        "a validated range failed when reopened for reverse traversal",
                    ));
                };
                let actual_reverse: Vec<_> = reverse_range
                    .rev()
                    .map(|(account_id, balance)| (*account_id, *balance))
                    .collect();
                let expected_reverse: Vec<_> = expected_entries.into_iter().rev().collect();
                prop_assert_eq!(actual_reverse, expected_reverse);
            }
            (Err(actual_error), None) => {
                return Err(TestCaseError::fail(format!(
                    "valid mathematical bounds returned {actual_error:?}"
                )));
            }
            (Ok(_range), Some(expected_error)) => {
                return Err(TestCaseError::fail(format!(
                    "invalid mathematical bounds did not return {expected_error:?}"
                )));
            }
        }

        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_ranges_match_mathematical_model_at_capacity_three(
            entries in collection::vec((generated_account_id(), generated_balance()), 0..96),
            start in generated_bound(),
            end in generated_bound(),
        ) {
            assert_ranges_match_standard::<3>(&entries, start, end)?;
        }

        #[test]
        fn generated_ranges_match_mathematical_model_at_capacity_four(
            entries in collection::vec((generated_account_id(), generated_balance()), 0..96),
            start in generated_bound(),
            end in generated_bound(),
        ) {
            assert_ranges_match_standard::<4>(&entries, start, end)?;
        }

        #[test]
        fn generated_ranges_match_mathematical_model_at_capacity_thirty_two(
            entries in collection::vec((generated_account_id(), generated_balance()), 0..96),
            start in generated_bound(),
            end in generated_bound(),
        ) {
            assert_ranges_match_standard::<32>(&entries, start, end)?;
        }
    }

    fn account_balances(account_ids: impl IntoIterator<Item = u64>) -> AccountBalances {
        let mut balances = AccountBalances::new();
        account_ids.into_iter().for_each(|account_id| {
            assert_eq!(
                balances.insert(format!("account-2026-{account_id}"), account_id * 10),
                InsertOutcome::Inserted
            );
        });
        balances
    }

    fn account_ids<'a>(entries: impl Iterator<Item = (&'a String, &'a u64)>) -> Vec<&'a str> {
        entries
            .map(|(account_id, _balance)| account_id.as_str())
            .collect()
    }

    #[test]
    fn iter_empty_tree_returns_none() {
        let balances = AccountBalances::new();

        let mut entries = balances.iter();

        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
    }

    #[test]
    fn iter_singleton_tree_yields_its_entry() {
        let balances = account_balances([1_001]);

        let entries = account_ids(balances.iter());

        assert_eq!(entries, ["account-2026-1001"]);
    }

    #[test]
    fn iter_advances_within_leaf() {
        let balances = account_balances([2_001, 1_001, 3_001]);

        let entries = account_ids(balances.iter());

        assert_eq!(
            entries,
            [
                "account-2026-1001",
                "account-2026-2001",
                "account-2026-3001"
            ]
        );
    }

    #[test]
    fn iter_crosses_leaf_boundary_in_order() {
        let balances = account_balances(1_001..=1_012);

        let entries = account_ids(balances.iter());

        assert_eq!(entries.len(), 12);
        assert!(entries.is_sorted());
        assert_eq!(entries.first(), Some(&"account-2026-1001"));
        assert_eq!(entries.last(), Some(&"account-2026-1012"));
    }

    #[test]
    fn iter_ascends_across_exhausted_branch() {
        let balances = account_balances(1_001..=1_064);

        let entries = account_ids(balances.iter());

        assert_eq!(entries.len(), 64);
        assert!(entries.is_sorted());
        assert_eq!(entries.last(), Some(&"account-2026-1064"));
    }

    #[test]
    fn iter_back_crosses_leaf_and_branch_boundaries_in_order() {
        let balances = account_balances(1_001..=1_064);

        let entries = account_ids(balances.iter().rev());

        assert_eq!(entries.len(), 64);
        assert!(entries.is_sorted_by(|left, right| left > right));
        assert_eq!(entries.first(), Some(&"account-2026-1064"));
        assert_eq!(entries.last(), Some(&"account-2026-1001"));
    }

    #[test]
    fn iter_reports_exact_size_hint() {
        let balances = account_balances(1_001..=1_012);
        let mut entries = balances.iter();

        let initial = entries.size_hint();
        let first = entries.next();
        let after_front = entries.size_hint();
        let last = entries.next_back();

        assert!(first.is_some());
        assert!(last.is_some());
        assert_eq!(initial, (12, Some(12)));
        assert_eq!(after_front, (11, Some(11)));
        assert_eq!(entries.size_hint(), (10, Some(10)));
        assert_eq!(entries.len(), 10);
    }

    #[test]
    fn iter_front_and_back_do_not_duplicate_entry() {
        let balances = account_balances(1_001..=1_005);
        let mut entries = balances.iter();

        let visited = [
            entries.next().map(|(key, _value)| key.as_str()),
            entries.next_back().map(|(key, _value)| key.as_str()),
            entries.next().map(|(key, _value)| key.as_str()),
            entries.next_back().map(|(key, _value)| key.as_str()),
            entries.next().map(|(key, _value)| key.as_str()),
        ];

        assert_eq!(
            visited,
            [
                Some("account-2026-1001"),
                Some("account-2026-1005"),
                Some("account-2026-1002"),
                Some("account-2026-1004"),
                Some("account-2026-1003"),
            ]
        );
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
    }

    #[test]
    fn iter_after_last_entry_is_fused() {
        let balances = account_balances([1_001]);
        let mut entries = balances.iter();

        assert!(entries.next().is_some());
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
    }

    #[test]
    fn range_with_ordered_bounds_is_valid() {
        let balances = account_balances(1_001..=1_008);

        let result = balances.range::<str, _>((
            Bound::Included("account-2026-1003"),
            Bound::Included("account-2026-1006"),
        ));

        assert!(result.is_ok());
    }

    #[test]
    fn range_with_reversed_bounds_returns_error() {
        let balances = account_balances(1_001..=1_008);

        let result = balances.range::<str, _>((
            Bound::Included("account-2026-1006"),
            Bound::Included("account-2026-1003"),
        ));

        assert!(matches!(result, Err(RangeError::StartAfterEnd)));
    }

    #[test]
    fn range_with_equal_included_bounds_yields_match() {
        let balances = account_balances(1_001..=1_008);

        let entries = balances
            .range::<str, _>((
                Bound::Included("account-2026-1004"),
                Bound::Included("account-2026-1004"),
            ))
            .map(account_ids);

        assert_eq!(entries, Ok(vec!["account-2026-1004"]));
    }

    #[test]
    fn range_with_equal_excluded_bound_returns_error() {
        let balances = account_balances(1_001..=1_008);

        let both_excluded = balances.range::<str, _>((
            Bound::Excluded("account-2026-1004"),
            Bound::Excluded("account-2026-1004"),
        ));
        let start_excluded = balances.range::<str, _>((
            Bound::Excluded("account-2026-1004"),
            Bound::Included("account-2026-1004"),
        ));
        let end_excluded = balances.range::<str, _>((
            Bound::Included("account-2026-1004"),
            Bound::Excluded("account-2026-1004"),
        ));

        assert!(matches!(
            both_excluded,
            Err(RangeError::EmptyExcludedBounds)
        ));
        assert!(matches!(
            start_excluded,
            Err(RangeError::EmptyExcludedBounds)
        ));
        assert!(matches!(end_excluded, Err(RangeError::EmptyExcludedBounds)));
    }

    #[test]
    fn range_with_unbounded_start_begins_at_first_key() {
        let balances = account_balances(1_001..=1_008);

        let entries = balances
            .range::<str, _>((Bound::Unbounded, Bound::Excluded("account-2026-1004")))
            .map(account_ids);

        assert_eq!(
            entries,
            Ok(vec![
                "account-2026-1001",
                "account-2026-1002",
                "account-2026-1003"
            ])
        );
    }

    #[test]
    fn range_excluded_end_at_leaf_minimum_keeps_preceding_leaf() {
        let balances = account_balances(1_001..=1_008);

        let entries = balances
            .range::<str, _>((Bound::Unbounded, Bound::Excluded("account-2026-1005")))
            .map(account_ids);

        assert_eq!(
            entries,
            Ok(vec![
                "account-2026-1001",
                "account-2026-1002",
                "account-2026-1003",
                "account-2026-1004"
            ])
        );
    }

    #[test]
    fn range_with_unbounded_end_finishes_at_last_key() {
        let balances = account_balances(1_001..=1_008);

        let entries = balances
            .range::<str, _>((Bound::Excluded("account-2026-1005"), Bound::Unbounded))
            .map(account_ids);

        assert_eq!(
            entries,
            Ok(vec![
                "account-2026-1006",
                "account-2026-1007",
                "account-2026-1008"
            ])
        );
    }

    #[test]
    fn range_endpoints_honor_inclusion_and_exclusion() {
        let balances = account_balances(1_001..=1_008);

        let included = balances
            .range::<str, _>((
                Bound::Included("account-2026-1003"),
                Bound::Included("account-2026-1006"),
            ))
            .map(account_ids);
        let excluded = balances
            .range::<str, _>((
                Bound::Excluded("account-2026-1003"),
                Bound::Excluded("account-2026-1006"),
            ))
            .map(account_ids);

        assert_eq!(
            included,
            Ok(vec![
                "account-2026-1003",
                "account-2026-1004",
                "account-2026-1005",
                "account-2026-1006"
            ])
        );
        assert_eq!(excluded, Ok(vec!["account-2026-1004", "account-2026-1005"]));
    }

    #[test]
    fn range_without_matching_accounts_is_empty() {
        let balances = account_balances([1_001, 2_001, 3_001]);

        let entries = balances
            .range::<str, _>((
                Bound::Included("account-2026-2100"),
                Bound::Included("account-2026-2900"),
            ))
            .map(account_ids);

        assert_eq!(entries, Ok(Vec::new()));
    }

    #[test]
    fn range_with_str_bounds_filters_string_keys() {
        let balances = account_balances(1_001..=1_012);

        let entries = balances
            .range::<str, _>((
                Bound::Included("account-2026-1009"),
                Bound::Excluded("account-2026-1012"),
            ))
            .map(account_ids);

        assert_eq!(
            entries,
            Ok(vec![
                "account-2026-1009",
                "account-2026-1010",
                "account-2026-1011"
            ])
        );
    }

    #[test]
    fn range_front_and_back_stop_when_crossed() {
        let balances = account_balances(1_001..=1_008);
        let mut entries = balances
            .range::<str, _>((
                Bound::Included("account-2026-1003"),
                Bound::Included("account-2026-1006"),
            ))
            .unwrap();

        let visited = [
            entries.next().map(|(key, _value)| key.as_str()),
            entries.next_back().map(|(key, _value)| key.as_str()),
            entries.next_back().map(|(key, _value)| key.as_str()),
            entries.next().map(|(key, _value)| key.as_str()),
        ];

        assert_eq!(
            visited,
            [
                Some("account-2026-1003"),
                Some("account-2026-1006"),
                Some("account-2026-1005"),
                Some("account-2026-1004"),
            ]
        );
        assert_eq!(entries.next(), None);
        assert_eq!(entries.next_back(), None);
        assert_eq!(entries.next_back(), None);
        assert_eq!(entries.next(), None);
    }
}
