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
mod tests {
    use super::BTree;

    type AccountBalances = BTree<String, u64, 3>;

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
}
