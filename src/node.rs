/// Compile-time node-capacity constraint.
pub(crate) struct NodeCapacity<const CAPACITY: usize>(());

impl<const CAPACITY: usize> NodeCapacity<CAPACITY> {
    const VALID: () = assert!(CAPACITY >= 3, "node capacity must be at least three");

    /// Triggers validation when a tree is instantiated.
    pub(crate) const fn validate() {
        let () = Self::VALID;
    }
}

/// Closed set of node shapes.
pub(crate) enum Node<K, V, const CAPACITY: usize> {
    Leaf(LeafNode<K, V, CAPACITY>),
}

impl<K, V, const CAPACITY: usize> Node<K, V, CAPACITY> {
    /// Creates the canonical empty root node.
    pub(crate) fn empty_leaf() -> Self {
        Self::Leaf(LeafNode::empty())
    }

    /// Returns the first leaf entry, if present.
    pub(crate) fn first_key_value(&self) -> Option<(&K, &V)> {
        match self {
            Self::Leaf(leaf) => leaf.first_key_value(),
        }
    }

    /// Returns the last leaf entry, if present.
    pub(crate) fn last_key_value(&self) -> Option<(&K, &V)> {
        match self {
            Self::Leaf(leaf) => leaf.last_key_value(),
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty_leaf(&self) -> bool {
        match self {
            Self::Leaf(leaf) => leaf.is_empty(),
        }
    }

    #[cfg(test)]
    pub(crate) fn allocated_entry_capacity(&self) -> usize {
        match self {
            Self::Leaf(leaf) => leaf.allocated_entry_capacity(),
        }
    }
}

/// Sorted entries owned by a leaf node.
pub(crate) struct LeafNode<K, V, const CAPACITY: usize> {
    entries: Vec<Entry<K, V>>,
}

impl<K, V, const CAPACITY: usize> LeafNode<K, V, CAPACITY> {
    /// Creates a leaf without allocating an entry buffer.
    fn empty() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Returns the first entry, if present.
    fn first_key_value(&self) -> Option<(&K, &V)> {
        self.entries.first().map(Entry::as_pair)
    }

    /// Returns the last entry, if present.
    fn last_key_value(&self) -> Option<(&K, &V)> {
        self.entries.last().map(Entry::as_pair)
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    fn allocated_entry_capacity(&self) -> usize {
        self.entries.capacity()
    }
}

struct Entry<K, V> {
    key: K,
    value: V,
}

impl<K, V> Entry<K, V> {
    fn as_pair(&self) -> (&K, &V) {
        (&self.key, &self.value)
    }
}
