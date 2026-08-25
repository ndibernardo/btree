use std::borrow::Borrow;

use crate::node::{Node, NodeCapacity};

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

    #[cfg(test)]
    fn is_empty_leaf(&self) -> bool {
        self.node.is_empty_leaf()
    }

    #[cfg(test)]
    fn allocated_entry_capacity(&self) -> usize {
        self.node.allocated_entry_capacity()
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
}

impl<K, V, const CAPACITY: usize> Default for BTree<K, V, CAPACITY> {
    fn default() -> Self {
        Self::new()
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
}

#[cfg(test)]
mod tests {
    use super::BTree;
    use crate::node::Node;

    type AccountBalances = BTree<String, u64, 3>;

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
}
