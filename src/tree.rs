use std::borrow::Borrow;
use std::iter::FromIterator;
use std::ops::RangeBounds;

use crate::iter::IntoIter;
use crate::iter::Iter;
use crate::iter::Range;
use crate::iter::RangeError;
use crate::node::InsertResult;
use crate::node::Node;
use crate::node::NodeCapacity;

/// Public result of inserting a key-value pair.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "insertion reports whether an existing value was replaced"]
pub enum InsertOutcome<V> {
    /// A new key was inserted.
    Inserted,
    /// An existing key's value was replaced.
    Replaced {
        /// Value previously associated with the key.
        previous: V,
    },
}

/// Public result of removing a key-value pair.
#[derive(Debug, PartialEq, Eq)]
#[must_use = "removal reports whether the key existed"]
pub enum RemoveOutcome<V> {
    /// The key existed and its value was removed.
    Removed {
        /// Value previously associated with the key.
        value: V,
    },
    /// The key was not present.
    Missing,
}

/// Number of entries stored by a tree.
#[derive(Clone, Copy)]
struct EntryCount(usize);

impl EntryCount {
    const fn empty() -> Self {
        Self(0)
    }

    const fn get(self) -> usize {
        self.0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn increment(&mut self) {
        self.0 += 1;
    }

    fn decrement(&mut self) {
        self.0 -= 1;
    }
}

/// Root node, whose occupancy rules differ from non-root nodes.
struct Root<K, V, const CAPACITY: usize> {
    node: Node<K, V, CAPACITY>,
}

impl<K, V, const CAPACITY: usize> Root<K, V, CAPACITY> {
    fn empty() -> Self {
        Self {
            node: Node::empty_leaf(),
        }
    }

    fn first_key_value(&self) -> Option<(&K, &V)> {
        self.node.first_key_value()
    }

    fn last_key_value(&self) -> Option<(&K, &V)> {
        self.node.last_key_value()
    }

    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.node.get(key)
    }

    fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.node.get_mut(key)
    }

    fn grow(&mut self, separator: K, right: Box<Node<K, V, CAPACITY>>) {
        let left = std::mem::replace(&mut self.node, Node::empty_leaf());
        self.node = Node::from_root_split(left, separator, right);
    }

    #[cfg(test)]
    fn is_empty_leaf(&self) -> bool {
        self.node.is_empty_leaf()
    }

    #[cfg(test)]
    fn allocated_entry_capacity(&self) -> usize {
        self.node.allocated_entry_capacity()
    }

    #[cfg(test)]
    fn height(&self) -> usize {
        self.node.height()
    }
}

impl<K: Ord + Clone, V, const CAPACITY: usize> Root<K, V, CAPACITY> {
    fn insert(&mut self, key: K, value: V) -> InsertResult<K, V, CAPACITY> {
        self.node.insert(key, value)
    }

    fn remove<Q>(&mut self, key: &Q) -> crate::node::RemoveResult<K, V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let result = self.node.remove(key);
        match &result {
            crate::node::RemoveResult::Missing => {}
            crate::node::RemoveResult::Removed { .. } => self.node.normalize_root(),
        }
        result
    }
}

/// Ordered map whose node capacity is part of its type.
///
/// `CAPACITY` is the maximum number of entries or separators in a stable node.
/// It must be at least three. The default is 32.
pub struct BTree<K, V, const CAPACITY: usize = 32> {
    root: Root<K, V, CAPACITY>,
    length: EntryCount,
}

impl<K, V, const CAPACITY: usize> BTree<K, V, CAPACITY> {
    /// Creates an empty tree without allocating an entry buffer.
    pub fn new() -> Self {
        NodeCapacity::<CAPACITY>::validate();

        Self {
            root: Root::empty(),
            length: EntryCount::empty(),
        }
    }

    /// Returns the number of stored entries.
    pub const fn len(&self) -> usize {
        self.length.get()
    }

    /// Returns whether the tree contains no entries.
    pub const fn is_empty(&self) -> bool {
        self.length.is_empty()
    }

    /// Removes all entries and restores the canonical empty root.
    pub fn clear(&mut self) {
        self.root = Root::empty();
        self.length = EntryCount::empty();
    }
}

impl<K: Ord, V, const CAPACITY: usize> BTree<K, V, CAPACITY> {
    /// Returns the value associated with an owned or borrowed key.
    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.root.get(key)
    }

    /// Returns a mutable value associated with an owned or borrowed key.
    pub fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.root.get_mut(key)
    }

    /// Returns whether the tree contains an owned or borrowed key.
    pub fn contains_key<Q>(&self, key: &Q) -> bool
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.get(key).is_some()
    }

    /// Returns the smallest key and its value, if present.
    pub fn first_key_value(&self) -> Option<(&K, &V)> {
        self.root.first_key_value()
    }

    /// Returns the greatest key and its value, if present.
    pub fn last_key_value(&self) -> Option<(&K, &V)> {
        self.root.last_key_value()
    }

    /// Returns an immutable iterator over all entries in key order.
    pub fn iter(&self) -> Iter<'_, K, V, CAPACITY> {
        Iter::new(&self.root.node, self.length.get())
    }

    /// Returns an immutable iterator over entries within the given bounds.
    ///
    /// # Errors
    ///
    /// Returns [`RangeError::StartAfterEnd`] when the start sorts after the end.
    /// Returns [`RangeError::EmptyExcludedBounds`] when equal bounds are not both included.
    pub fn range<Q, R>(&self, bounds: R) -> Result<Range<'_, K, V, CAPACITY>, RangeError>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
        R: RangeBounds<Q>,
    {
        Range::new(&self.root.node, &bounds)
    }
}

impl<K: Ord + Clone, V, const CAPACITY: usize> BTree<K, V, CAPACITY> {
    /// Inserts a key-value pair, returning whether a value was replaced.
    pub fn insert(&mut self, key: K, value: V) -> InsertOutcome<V> {
        match self.root.insert(key, value) {
            InsertResult::Inserted => {
                self.length.increment();
                InsertOutcome::Inserted
            }
            InsertResult::Replaced { previous } => InsertOutcome::Replaced { previous },
            InsertResult::InsertedAndSplit { separator, right } => {
                self.length.increment();
                self.root.grow(separator, right);
                InsertOutcome::Inserted
            }
        }
    }

    /// Removes a key and returns its value when present.
    pub fn remove<Q>(&mut self, key: &Q) -> RemoveOutcome<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.root.remove(key) {
            crate::node::RemoveResult::Missing => RemoveOutcome::Missing,
            crate::node::RemoveResult::Removed {
                value,
                occupancy: _occupancy,
            } => {
                self.length.decrement();
                RemoveOutcome::Removed { value }
            }
        }
    }

    /// Removes and returns the least key-value pair, if present.
    pub fn pop_first(&mut self) -> Option<(K, V)> {
        let key = self.first_key_value()?.0.clone();

        match self.remove(&key) {
            RemoveOutcome::Removed { value } => Some((key, value)),
            RemoveOutcome::Missing => {
                unreachable!("the least key remains present until its immediate removal")
            }
        }
    }

    /// Removes and returns the greatest key-value pair, if present.
    pub fn pop_last(&mut self) -> Option<(K, V)> {
        let key = self.last_key_value()?.0.clone();

        match self.remove(&key) {
            RemoveOutcome::Removed { value } => Some((key, value)),
            RemoveOutcome::Missing => {
                unreachable!("the greatest key remains present until its immediate removal")
            }
        }
    }
}

impl<K, V, const CAPACITY: usize> Default for BTree<K, V, CAPACITY> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + std::fmt::Debug, V: std::fmt::Debug, const CAPACITY: usize> std::fmt::Debug
    for BTree<K, V, CAPACITY>
{
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_map().entries(self.iter()).finish()
    }
}

impl<'tree, K: Ord, V, const CAPACITY: usize> IntoIterator for &'tree BTree<K, V, CAPACITY> {
    type Item = (&'tree K, &'tree V);
    type IntoIter = Iter<'tree, K, V, CAPACITY>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<K, V, const CAPACITY: usize> IntoIterator for BTree<K, V, CAPACITY> {
    type Item = (K, V);
    type IntoIter = IntoIter<K, V>;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter::new(self.root.node, self.length.get())
    }
}

impl<K: Ord + Clone, V, const CAPACITY: usize> Extend<(K, V)> for BTree<K, V, CAPACITY> {
    fn extend<T: IntoIterator<Item = (K, V)>>(&mut self, entries: T) {
        entries.into_iter().for_each(|(key, value)| {
            let _outcome = self.insert(key, value);
        });
    }
}

impl<K: Ord + Clone, V, const CAPACITY: usize> FromIterator<(K, V)> for BTree<K, V, CAPACITY> {
    fn from_iter<T: IntoIterator<Item = (K, V)>>(entries: T) -> Self {
        let mut tree = Self::new();
        tree.extend(entries);
        tree
    }
}

#[cfg(test)]
impl<K, V, const CAPACITY: usize> BTree<K, V, CAPACITY> {
    fn from_test_root(node: Node<K, V, CAPACITY>) -> Self {
        NodeCapacity::<CAPACITY>::validate();
        let length = EntryCount(node.entry_count());

        Self {
            root: Root { node },
            length,
        }
    }

    fn assert_valid(&self)
    where
        K: Ord + std::fmt::Debug,
    {
        let counted_entries = self.root.node.assert_valid_root();
        assert_eq!(
            counted_entries,
            self.length.get(),
            "stored length matches leaf total"
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap as StandardBTreeMap;

    use proptest::collection;
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;
    use proptest::test_runner::TestCaseResult;

    use super::BTree;
    use super::EntryCount;
    use super::InsertOutcome;
    use super::RemoveOutcome;
    use crate::node::Node;

    type AccountBalances = BTree<String, u64, 3>;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct GeneratedAccountId(u16);

    impl GeneratedAccountId {
        const fn new(value: u16) -> Self {
            Self(value)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct GeneratedBalanceCents(u64);

    impl GeneratedBalanceCents {
        const fn new(value: u64) -> Self {
            Self(value)
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum MapOperation {
        Insert {
            account_id: GeneratedAccountId,
            balance: GeneratedBalanceCents,
        },
        Remove {
            account_id: GeneratedAccountId,
        },
        Lookup {
            account_id: GeneratedAccountId,
        },
        AdjustBalance {
            account_id: GeneratedAccountId,
            new_balance: GeneratedBalanceCents,
        },
        Clear,
    }

    fn generated_account_id() -> impl Strategy<Value = GeneratedAccountId> {
        (1_001_u16..1_129).prop_map(GeneratedAccountId::new)
    }

    fn generated_balance() -> impl Strategy<Value = GeneratedBalanceCents> {
        (100_u64..10_000_000).prop_map(GeneratedBalanceCents::new)
    }

    fn map_operation() -> impl Strategy<Value = MapOperation> {
        prop_oneof![
            4 => (generated_account_id(), generated_balance()).prop_map(
                |(account_id, balance)| MapOperation::Insert {
                    account_id,
                    balance,
                },
            ),
            3 => generated_account_id().prop_map(|account_id| MapOperation::Remove {
                account_id,
            }),
            2 => generated_account_id().prop_map(|account_id| MapOperation::Lookup {
                account_id,
            }),
            2 => (generated_account_id(), generated_balance()).prop_map(
                |(account_id, new_balance)| MapOperation::AdjustBalance {
                    account_id,
                    new_balance,
                },
            ),
            1 => Just(MapOperation::Clear),
        ]
    }

    fn assert_operations_match_standard<const CAPACITY: usize>(
        operations: &[MapOperation],
    ) -> TestCaseResult {
        let mut actual = BTree::<GeneratedAccountId, GeneratedBalanceCents, CAPACITY>::new();
        let mut expected = StandardBTreeMap::new();

        operations.iter().copied().try_for_each(|operation| {
            match operation {
                MapOperation::Insert {
                    account_id,
                    balance,
                } => {
                    let actual_outcome = actual.insert(account_id, balance);
                    let expected_previous = expected.insert(account_id, balance);

                    match (actual_outcome, expected_previous) {
                        (InsertOutcome::Inserted, None) => {}
                        (
                            InsertOutcome::Replaced {
                                previous: actual_previous,
                            },
                            Some(expected_previous),
                        ) => prop_assert_eq!(actual_previous, expected_previous),
                        (actual_outcome, expected_previous) => {
                            return Err(TestCaseError::fail(format!(
                                "insert outcome differs: {actual_outcome:?} versus {expected_previous:?}"
                            )));
                        }
                    }
                }
                MapOperation::Remove { account_id } => {
                    let actual_outcome = actual.remove(&account_id);
                    let expected_value = expected.remove(&account_id);

                    match (actual_outcome, expected_value) {
                        (RemoveOutcome::Missing, None) => {}
                        (
                            RemoveOutcome::Removed {
                                value: actual_value,
                            },
                            Some(expected_value),
                        ) => prop_assert_eq!(actual_value, expected_value),
                        (actual_outcome, expected_value) => {
                            return Err(TestCaseError::fail(format!(
                                "remove outcome differs: {actual_outcome:?} versus {expected_value:?}"
                            )));
                        }
                    }
                }
                MapOperation::Lookup { account_id } => {
                    prop_assert_eq!(actual.get(&account_id), expected.get(&account_id));
                    prop_assert_eq!(
                        actual.contains_key(&account_id),
                        expected.contains_key(&account_id)
                    );
                }
                MapOperation::AdjustBalance {
                    account_id,
                    new_balance,
                } => match (
                    actual.get_mut(&account_id),
                    expected.get_mut(&account_id),
                ) {
                    (Some(actual_balance), Some(expected_balance)) => {
                        prop_assert_eq!(*actual_balance, *expected_balance);
                        *actual_balance = new_balance;
                        *expected_balance = new_balance;
                    }
                    (None, None) => {}
                    (actual_balance, expected_balance) => {
                        return Err(TestCaseError::fail(format!(
                            "mutable lookup presence differs: {} versus {}",
                            actual_balance.is_some(),
                            expected_balance.is_some()
                        )));
                    }
                },
                MapOperation::Clear => {
                    actual.clear();
                    expected.clear();
                }
            }

            actual.assert_valid();
            prop_assert_eq!(actual.len(), expected.len());
            prop_assert_eq!(actual.first_key_value(), expected.first_key_value());
            prop_assert_eq!(actual.last_key_value(), expected.last_key_value());
            prop_assert!(actual.iter().eq(expected.iter()));
            prop_assert!(actual.iter().rev().eq(expected.iter().rev()));

            Ok(())
        })
    }

    #[test]
    fn mutable_lookups_match_standard_map_for_present_and_missing_accounts() -> TestCaseResult {
        assert_operations_match_standard::<3>(&[
            MapOperation::Insert {
                account_id: GeneratedAccountId::new(1_001),
                balance: GeneratedBalanceCents::new(12_500),
            },
            MapOperation::AdjustBalance {
                account_id: GeneratedAccountId::new(1_001),
                new_balance: GeneratedBalanceCents::new(15_000),
            },
            MapOperation::AdjustBalance {
                account_id: GeneratedAccountId::new(2_001),
                new_balance: GeneratedBalanceCents::new(25_000),
            },
        ])
    }

    #[test]
    #[should_panic(expected = "stored length matches leaf total")]
    fn validate_rejects_incorrect_entry_count() {
        let mut balances = leaf_balances();
        balances.length = EntryCount(2);

        balances.assert_valid();
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(128))]

        #[test]
        fn generated_operations_match_standard_map_at_capacity_three(
            operations in collection::vec(map_operation(), 0..128),
        ) {
            assert_operations_match_standard::<3>(&operations)?;
        }

        #[test]
        fn generated_operations_match_standard_map_at_capacity_four(
            operations in collection::vec(map_operation(), 0..128),
        ) {
            assert_operations_match_standard::<4>(&operations)?;
        }

        #[test]
        fn generated_operations_match_standard_map_at_capacity_five(
            operations in collection::vec(map_operation(), 0..128),
        ) {
            assert_operations_match_standard::<5>(&operations)?;
        }

        #[test]
        fn generated_operations_match_standard_map_at_capacity_eight(
            operations in collection::vec(map_operation(), 0..128),
        ) {
            assert_operations_match_standard::<8>(&operations)?;
        }

        #[test]
        fn generated_operations_match_standard_map_at_capacity_thirty_two(
            operations in collection::vec(map_operation(), 0..128),
        ) {
            assert_operations_match_standard::<32>(&operations)?;
        }
    }

    fn leaf_balances() -> AccountBalances {
        AccountBalances::from_test_root(Node::from_sorted_entries([
            (String::from("account-2026-1001"), 12_500),
            (String::from("account-2026-2001"), 25_000),
            (String::from("account-2026-3001"), 37_500),
        ]))
    }

    fn branched_balances() -> AccountBalances {
        AccountBalances::from_test_root(Node::from_sorted_branch(
            Node::from_sorted_entries([
                (String::from("account-2026-1001"), 12_500),
                (String::from("account-2026-1501"), 18_750),
            ]),
            (
                String::from("account-2026-2001"),
                Node::from_sorted_entries([
                    (String::from("account-2026-2001"), 25_000),
                    (String::from("account-2026-2501"), 31_250),
                ]),
            ),
            [(
                String::from("account-2026-3001"),
                Node::from_sorted_entries([
                    (String::from("account-2026-3001"), 37_500),
                    (String::from("account-2026-3501"), 43_750),
                ]),
            )],
        ))
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

    fn insert_accounts_and_validate(account_ids: impl IntoIterator<Item = u64>) {
        let mut balances = AccountBalances::new();
        let mut inserted_accounts = Vec::new();

        account_ids.into_iter().for_each(|account_id| {
            let key = format!("account-2026-{account_id}");
            let value = account_id * 10;
            let outcome = balances.insert(key.clone(), value);
            inserted_accounts.push((key, value));

            assert_eq!(outcome, InsertOutcome::Inserted);
            balances.assert_valid();
            assert_eq!(balances.len(), inserted_accounts.len());
            inserted_accounts
                .iter()
                .for_each(|(inserted_key, balance)| {
                    assert_eq!(balances.get(inserted_key), Some(balance));
                });
        });
    }

    fn remove_accounts_and_validate(account_ids: impl IntoIterator<Item = u64>) {
        let mut balances = AccountBalances::new();
        let mut retained = (1_001_u64..=1_064).collect::<Vec<_>>();
        retained.iter().for_each(|account_id| {
            assert_eq!(
                balances.insert(format!("account-2026-{account_id}"), account_id * 10),
                InsertOutcome::Inserted
            );
        });

        account_ids.into_iter().for_each(|account_id| {
            let outcome = balances.remove(format!("account-2026-{account_id}").as_str());
            let retained_index = retained
                .binary_search(&account_id)
                .expect("removal order must contain each inserted account exactly once");
            retained.remove(retained_index);

            assert_eq!(
                outcome,
                RemoveOutcome::Removed {
                    value: account_id * 10
                }
            );
            balances.assert_valid();
            assert_eq!(balances.len(), retained.len());
            assert_eq!(
                balances
                    .iter()
                    .map(|(key, _value)| key.as_str())
                    .collect::<Vec<_>>(),
                retained
                    .iter()
                    .map(|retained_id| format!("account-2026-{retained_id}"))
                    .collect::<Vec<_>>()
            );
        });
    }

    #[test]
    fn new_with_valid_capacity_creates_empty_tree() {
        let balances = AccountBalances::new();

        assert!(balances.is_empty());
        assert_eq!(balances.len(), 0);
    }

    #[test]
    fn new_creates_empty_leaf_root() {
        let balances = AccountBalances::new();

        assert!(balances.root.is_empty_leaf());
        assert_eq!(balances.root.allocated_entry_capacity(), 0);
    }

    #[test]
    fn default_matches_new() {
        let default_balances = AccountBalances::default();
        let new_balances = AccountBalances::new();

        assert_eq!(default_balances.len(), new_balances.len());
        assert_eq!(default_balances.is_empty(), new_balances.is_empty());
        assert!(default_balances.root.is_empty_leaf());
        assert!(new_balances.root.is_empty_leaf());
    }

    #[test]
    fn clear_on_empty_tree_preserves_empty_root() {
        let mut balances = AccountBalances::new();

        balances.clear();

        assert!(balances.is_empty());
        assert_eq!(balances.len(), 0);
        assert!(balances.root.is_empty_leaf());
        assert_eq!(balances.root.allocated_entry_capacity(), 0);
        balances.assert_valid();
    }

    #[test]
    fn clear_on_populated_tree_restores_empty_root() {
        let mut balances = account_balances(1_001..=1_064);

        balances.clear();

        assert!(balances.is_empty());
        assert_eq!(balances.len(), 0);
        assert_eq!(balances.first_key_value(), None);
        assert_eq!(balances.last_key_value(), None);
        assert_eq!(balances.iter().next(), None);
        assert!(balances.root.is_empty_leaf());
        assert_eq!(balances.root.allocated_entry_capacity(), 0);
        balances.assert_valid();
    }

    #[test]
    fn extend_adds_new_account_balances() {
        let mut balances = account_balances([1_001]);

        balances.extend([
            (String::from("account-2026-3001"), 37_500),
            (String::from("account-2026-2001"), 25_000),
        ]);

        assert_eq!(balances.len(), 3);
        assert_eq!(
            balances
                .iter()
                .map(|(account_id, balance)| (account_id.as_str(), *balance))
                .collect::<Vec<_>>(),
            [
                ("account-2026-1001", 10_010),
                ("account-2026-2001", 25_000),
                ("account-2026-3001", 37_500),
            ]
        );
        balances.assert_valid();
    }

    #[test]
    fn extend_duplicate_account_keeps_last_balance() {
        let mut balances = account_balances([1_001]);

        balances.extend([
            (String::from("account-2026-1001"), 12_500),
            (String::from("account-2026-1001"), 15_000),
        ]);

        assert_eq!(balances.len(), 1);
        assert_eq!(balances.get("account-2026-1001"), Some(&15_000));
        balances.assert_valid();
    }

    #[test]
    fn from_iter_builds_ordered_account_map() {
        let balances: AccountBalances = [
            (String::from("account-2026-3001"), 37_500),
            (String::from("account-2026-1001"), 12_500),
            (String::from("account-2026-2001"), 25_000),
        ]
        .into_iter()
        .collect();

        assert_eq!(
            balances
                .iter()
                .map(|(account_id, balance)| (account_id.as_str(), *balance))
                .collect::<Vec<_>>(),
            [
                ("account-2026-1001", 12_500),
                ("account-2026-2001", 25_000),
                ("account-2026-3001", 37_500),
            ]
        );
        balances.assert_valid();
    }

    #[test]
    fn from_iter_duplicate_account_keeps_last_balance() {
        let balances: AccountBalances = [
            (String::from("account-2026-1001"), 12_500),
            (String::from("account-2026-1001"), 15_000),
        ]
        .into_iter()
        .collect();

        assert_eq!(balances.len(), 1);
        assert_eq!(balances.get("account-2026-1001"), Some(&15_000));
        balances.assert_valid();
    }

    #[test]
    fn shared_into_iter_yields_borrowed_entries() {
        let balances = account_balances([3_001, 1_001, 2_001]);

        let entries = (&balances)
            .into_iter()
            .map(|(account_id, balance)| (account_id.as_str(), *balance))
            .collect::<Vec<_>>();

        assert_eq!(
            entries,
            [
                ("account-2026-1001", 10_010),
                ("account-2026-2001", 20_010),
                ("account-2026-3001", 30_010),
            ]
        );
        assert_eq!(balances.get("account-2026-2001"), Some(&20_010));
    }

    #[test]
    fn owned_into_iter_yields_owned_entries() {
        let balances = account_balances([3_001, 1_001, 4_001, 2_001]);

        let entries = balances.into_iter().collect::<Vec<_>>();

        assert_eq!(
            entries,
            [
                (String::from("account-2026-1001"), 10_010),
                (String::from("account-2026-2001"), 20_010),
                (String::from("account-2026-3001"), 30_010),
                (String::from("account-2026-4001"), 40_010),
            ]
        );
    }

    #[test]
    fn debug_formats_entries_as_an_ordered_map() {
        let balances = account_balances([3_001, 1_001, 2_001]);

        let formatted = format!("{balances:?}");

        assert_eq!(
            formatted,
            "{\"account-2026-1001\": 10010, \"account-2026-2001\": 20010, \"account-2026-3001\": 30010}"
        );
    }

    #[test]
    fn empty_tree_accessors_report_absence() {
        let balances = AccountBalances::new();

        let first = balances.first_key_value();
        let last = balances.last_key_value();

        assert_eq!(first, None);
        assert_eq!(last, None);
    }

    #[test]
    fn get_on_empty_tree_returns_none() {
        let balances = AccountBalances::new();

        assert_eq!(balances.get("account-2026-1001"), None);
    }

    #[test]
    fn get_existing_account_returns_balance() {
        let balances = leaf_balances();

        assert_eq!(balances.get("account-2026-2001"), Some(&25_000));
    }

    #[test]
    fn get_missing_account_returns_none() {
        let balances = leaf_balances();

        assert_eq!(balances.get("account-2026-2501"), None);
    }

    #[test]
    fn get_with_str_finds_owned_string_key() {
        let balances = leaf_balances();
        let borrowed_account_id: &str = "account-2026-3001";

        assert_eq!(balances.get(borrowed_account_id), Some(&37_500));
    }

    #[test]
    fn get_in_multilevel_tree_follows_branch_path() {
        let balances = branched_balances();

        assert_eq!(balances.get("account-2026-3501"), Some(&43_750));
    }

    #[test]
    fn branched_tree_accessors_follow_outer_leaves() {
        let balances = branched_balances();

        assert_eq!(
            balances.first_key_value(),
            Some((&String::from("account-2026-1001"), &12_500))
        );
        assert_eq!(
            balances.last_key_value(),
            Some((&String::from("account-2026-3501"), &43_750))
        );
    }

    #[test]
    fn pop_first_on_empty_tree_returns_none() {
        let mut balances = AccountBalances::new();

        let popped = balances.pop_first();

        assert_eq!(popped, None);
        assert!(balances.root.is_empty_leaf());
        balances.assert_valid();
    }

    #[test]
    fn pop_first_removes_least_entry() {
        let mut balances = branched_balances();

        let popped = balances.pop_first();

        assert_eq!(popped, Some((String::from("account-2026-1001"), 12_500)));
        assert_eq!(balances.len(), 5);
        assert_eq!(
            balances.first_key_value(),
            Some((&String::from("account-2026-1501"), &18_750))
        );
        balances.assert_valid();
    }

    #[test]
    fn pop_last_on_empty_tree_returns_none() {
        let mut balances = AccountBalances::new();

        let popped = balances.pop_last();

        assert_eq!(popped, None);
        assert!(balances.root.is_empty_leaf());
        balances.assert_valid();
    }

    #[test]
    fn pop_last_removes_greatest_entry() {
        let mut balances = branched_balances();

        let popped = balances.pop_last();

        assert_eq!(popped, Some((String::from("account-2026-3501"), 43_750)));
        assert_eq!(balances.len(), 5);
        assert_eq!(
            balances.last_key_value(),
            Some((&String::from("account-2026-3001"), &37_500))
        );
        balances.assert_valid();
    }

    #[test]
    fn get_mut_updates_only_selected_account_balance() {
        let mut balances = branched_balances();

        let updated = balances.get_mut("account-2026-2501").map(|balance| {
            *balance = 32_000;
        });

        assert_eq!(updated, Some(()));
        assert_eq!(balances.get("account-2026-2501"), Some(&32_000));
        assert_eq!(balances.get("account-2026-2001"), Some(&25_000));
        assert_eq!(balances.get("account-2026-3001"), Some(&37_500));
    }

    #[test]
    fn get_mut_missing_account_preserves_tree() {
        let mut balances = leaf_balances();

        let missing = balances.get_mut("account-2026-2501");

        assert_eq!(missing, None);
        assert_eq!(
            balances.first_key_value(),
            Some((&String::from("account-2026-1001"), &12_500))
        );
        assert_eq!(
            balances.last_key_value(),
            Some((&String::from("account-2026-3001"), &37_500))
        );
    }

    #[test]
    fn contains_key_for_existing_account_returns_true() {
        let balances = leaf_balances();

        assert!(balances.contains_key("account-2026-1001"));
    }

    #[test]
    fn contains_key_for_missing_account_returns_false() {
        let balances = leaf_balances();

        assert!(!balances.contains_key("account-2026-2501"));
    }

    #[test]
    fn insert_into_empty_tree_returns_inserted() {
        let mut balances = AccountBalances::new();

        let outcome = balances.insert(String::from("account-2026-1001"), 12_500);

        assert_eq!(outcome, InsertOutcome::Inserted);
        assert_eq!(balances.len(), 1);
        assert_eq!(balances.get("account-2026-1001"), Some(&12_500));
    }

    #[test]
    fn insert_existing_account_returns_previous_balance_and_preserves_length() {
        let mut balances = AccountBalances::new();
        let inserted = balances.insert(String::from("account-2026-1001"), 12_500);

        let replaced = balances.insert(String::from("account-2026-1001"), 15_000);

        assert_eq!(inserted, InsertOutcome::Inserted);
        assert_eq!(replaced, InsertOutcome::Replaced { previous: 12_500 });
        assert_eq!(balances.len(), 1);
        assert_eq!(balances.get("account-2026-1001"), Some(&15_000));
    }

    #[test]
    fn stable_root_insertion_preserves_height() {
        let mut balances = AccountBalances::new();

        [1_001_u64, 2_001, 3_001].into_iter().for_each(|account| {
            let outcome = balances.insert(format!("account-2026-{account}"), account * 10);
            assert_eq!(outcome, InsertOutcome::Inserted);
        });

        assert_eq!(balances.root.height(), 0);
        assert_eq!(balances.len(), 3);
    }

    #[test]
    fn leaf_root_split_increases_height() {
        let mut balances = AccountBalances::new();

        [1_001_u64, 2_001, 3_001, 4_001]
            .into_iter()
            .for_each(|account| {
                let outcome = balances.insert(format!("account-2026-{account}"), account * 10);
                assert_eq!(outcome, InsertOutcome::Inserted);
            });

        assert_eq!(balances.root.height(), 1);
        assert_eq!(balances.len(), 4);
        assert_eq!(balances.get("account-2026-1001"), Some(&10_010));
        assert_eq!(balances.get("account-2026-4001"), Some(&40_010));
    }

    #[test]
    fn branch_root_split_increases_height() {
        let mut balances = AccountBalances::new();

        (1_u64..=10).for_each(|sequence| {
            let account = 1_000 + sequence;
            let outcome = balances.insert(format!("account-2026-{account}"), account * 10);
            assert_eq!(outcome, InsertOutcome::Inserted);
        });

        assert_eq!(balances.root.height(), 2);
        assert_eq!(balances.len(), 10);
        assert_eq!(balances.get("account-2026-1001"), Some(&10_010));
        assert_eq!(balances.get("account-2026-1010"), Some(&10_100));
    }

    #[test]
    fn insert_new_minimum_preserves_ancestor_routing() {
        let mut balances = AccountBalances::new();
        (1_u64..=10).for_each(|sequence| {
            let account = 1_000 + sequence;
            let outcome = balances.insert(format!("account-2026-{account}"), account * 10);
            assert_eq!(outcome, InsertOutcome::Inserted);
        });

        let outcome = balances.insert(String::from("account-2026-0501"), 5_010);

        assert_eq!(outcome, InsertOutcome::Inserted);
        assert_eq!(balances.len(), 11);
        balances.assert_valid();
        assert_eq!(
            balances.first_key_value(),
            Some((&String::from("account-2026-0501"), &5_010))
        );
        assert_eq!(balances.root.height(), 2);
        assert_eq!(balances.get("account-2026-1010"), Some(&10_100));
    }

    #[test]
    fn ascending_insertions_preserve_tree_invariants() {
        insert_accounts_and_validate(1_001_u64..=1_064);
    }

    #[test]
    fn descending_insertions_preserve_tree_invariants() {
        insert_accounts_and_validate((1_001_u64..=1_064).rev());
    }

    #[test]
    fn interleaved_insertions_preserve_tree_invariants() {
        let account_ids = (0_u64..32).flat_map(|offset| [1_001 + offset, 1_064 - offset]);

        insert_accounts_and_validate(account_ids);
    }

    #[test]
    fn remove_missing_account_preserves_tree() {
        let mut balances = leaf_balances();

        let outcome = balances.remove("account-2026-2501");

        assert_eq!(outcome, RemoveOutcome::Missing);
        assert_eq!(balances.len(), 3);
        assert_eq!(balances.get("account-2026-2001"), Some(&25_000));
        balances.assert_valid();
    }

    #[test]
    fn remove_from_occupied_root_leaf_returns_value() {
        let mut balances = leaf_balances();

        let outcome = balances.remove("account-2026-2001");

        assert_eq!(outcome, RemoveOutcome::Removed { value: 25_000 });
        assert_eq!(balances.len(), 2);
        assert!(!balances.contains_key("account-2026-2001"));
        balances.assert_valid();
    }

    #[test]
    fn remove_with_str_finds_owned_string_key() {
        let mut balances = leaf_balances();
        let borrowed_account_id: &str = "account-2026-3001";

        let outcome = balances.remove(borrowed_account_id);

        assert_eq!(outcome, RemoveOutcome::Removed { value: 37_500 });
        assert_eq!(
            balances.last_key_value().map(|(key, _value)| key.as_str()),
            Some("account-2026-2001")
        );
    }

    #[test]
    fn remove_last_account_restores_empty_root() {
        let mut balances = AccountBalances::new();
        let inserted = balances.insert(String::from("account-2026-1001"), 12_500);

        let outcome = balances.remove("account-2026-1001");

        assert_eq!(inserted, InsertOutcome::Inserted);
        assert_eq!(outcome, RemoveOutcome::Removed { value: 12_500 });
        assert!(balances.is_empty());
        assert!(balances.root.is_empty_leaf());
        balances.assert_valid();
    }

    #[test]
    fn single_child_root_collapses_one_level() {
        let mut balances = account_balances([1_001, 2_001, 3_001, 4_001]);
        assert_eq!(balances.root.height(), 1);

        let outcome = balances.remove("account-2026-4001");

        assert_eq!(outcome, RemoveOutcome::Removed { value: 40_010 });
        assert_eq!(balances.root.height(), 0);
        assert_eq!(balances.len(), 3);
        balances.assert_valid();
    }

    #[test]
    fn occupied_root_branch_preserves_height() {
        let mut balances = branched_balances();
        assert_eq!(balances.root.height(), 1);

        let outcome = balances.remove("account-2026-3501");

        assert_eq!(outcome, RemoveOutcome::Removed { value: 43_750 });
        assert_eq!(balances.root.height(), 1);
        assert_eq!(balances.len(), 5);
        assert!(!balances.contains_key("account-2026-3501"));
        balances.assert_valid();
    }

    #[test]
    fn remove_new_minimum_refreshes_ancestor_routing() {
        let mut balances = account_balances(1_001..=1_032);

        let outcome = balances.remove("account-2026-1001");

        assert_eq!(outcome, RemoveOutcome::Removed { value: 10_010 });
        assert_eq!(
            balances.first_key_value().map(|(key, _value)| key.as_str()),
            Some("account-2026-1002")
        );
        assert_eq!(balances.get("account-2026-1002"), Some(&10_020));
        balances.assert_valid();
    }

    #[test]
    fn ascending_removals_preserve_tree_invariants() {
        remove_accounts_and_validate(1_001_u64..=1_064);
    }

    #[test]
    fn descending_removals_preserve_tree_invariants() {
        remove_accounts_and_validate((1_001_u64..=1_064).rev());
    }

    #[test]
    fn interleaved_removals_preserve_tree_invariants() {
        let account_ids = (0_u64..32).flat_map(|offset| [1_001 + offset, 1_064 - offset]);

        remove_accounts_and_validate(account_ids);
    }

    #[test]
    fn even_capacity_removals_preserve_tree_invariants() {
        let mut balances = BTree::<String, u64, 4>::new();
        (1_001_u64..=1_096).for_each(|account_id| {
            assert_eq!(
                balances.insert(format!("account-2026-{account_id}"), account_id * 10),
                InsertOutcome::Inserted
            );
        });
        let removal_order = (0_u64..48).flat_map(|offset| [1_001 + offset, 1_096 - offset]);

        removal_order.for_each(|account_id| {
            assert_eq!(
                balances.remove(format!("account-2026-{account_id}").as_str()),
                RemoveOutcome::Removed {
                    value: account_id * 10
                }
            );
            balances.assert_valid();
        });

        assert!(balances.is_empty());
        assert!(balances.root.is_empty_leaf());
    }

    #[test]
    fn mixed_mutations_match_standard_btree_map() {
        let mut balances = AccountBalances::new();
        let mut expected = StandardBTreeMap::new();

        (0_u64..256).for_each(|sequence| {
            let account_id = 1_001 + (sequence * 37) % 97;
            let key = format!("account-2026-{account_id}");
            let balance = account_id * 10 + sequence;

            match sequence % 3 {
                0 | 1 => {
                    let actual = balances.insert(key.clone(), balance);
                    let standard = expected.insert(key.clone(), balance);
                    match standard {
                        Some(previous) => {
                            assert_eq!(actual, InsertOutcome::Replaced { previous });
                        }
                        None => assert_eq!(actual, InsertOutcome::Inserted),
                    }
                }
                2 => {
                    let actual = balances.remove(key.as_str());
                    let standard = expected.remove(key.as_str());
                    match standard {
                        Some(value) => {
                            assert_eq!(actual, RemoveOutcome::Removed { value });
                        }
                        None => assert_eq!(actual, RemoveOutcome::Missing),
                    }
                }
                _unreachable_remainder => unreachable!("remainder modulo three is at most two"),
            }

            balances.assert_valid();
            assert_eq!(balances.len(), expected.len());
            assert!(balances.iter().eq(expected.iter()));
        });
    }
}
