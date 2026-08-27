//! B+-tree representation and structural mutation.
//!
//! Values are owned only by leaves. A branch owns its leftmost child separately;
//! every later child is paired with a separator equal to that child's minimum
//! key. Stable leaves contain at most `CAPACITY` entries, stable branches contain
//! at most `CAPACITY` separator-child edges, and all leaves have the same depth.
//!
//! Insertion may create one overflow slot before a split. Removal may create an
//! underfull node or a one-child branch while typed outcomes propagate upward;
//! root normalization restores the stable representation before the public
//! operation returns. Keys are cloned only when the minimum key duplicated by a
//! branch separator must be created or refreshed.

use std::borrow::Borrow;
use std::cmp::Ordering;

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
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Node<K, V, const CAPACITY: usize> {
    Leaf(LeafNode<K, V, CAPACITY>),
    Branch(BranchNode<K, V, CAPACITY>),
}

/// Result of inserting into a subtree.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum InsertResult<K, V, const CAPACITY: usize> {
    Inserted,
    Replaced {
        previous: V,
    },
    InsertedAndSplit {
        separator: K,
        right: Box<Node<K, V, CAPACITY>>,
    },
}

/// Result of removing from a subtree.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RemoveResult<K, V> {
    Missing,
    Removed {
        value: V,
        occupancy: OccupancyChange<K>,
    },
}

/// Occupancy and minimum-key change after removal.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum OccupancyChange<K> {
    Stable { minimum: MinimumChange<K> },
    Underflow { minimum: MinimumChange<K> },
}

/// Change to a subtree's minimum key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MinimumChange<K> {
    Unchanged,
    Changed(K),
    Removed,
}

impl<K: Ord + Clone> MinimumChange<K> {
    fn between(previous: Option<K>, current: Option<&K>) -> Self {
        match (previous, current) {
            (Some(previous), Some(current)) if &previous == current => Self::Unchanged,
            (Some(_previous), Some(current)) => Self::Changed(current.clone()),
            (None, Some(current)) => Self::Changed(current.clone()),
            (Some(_previous), None) => Self::Removed,
            (None, None) => Self::Unchanged,
        }
    }
}

/// Leaf- or branch-local removal before occupancy is classified.
enum Removal<V> {
    Missing,
    Removed(V),
}

type AdjacentChildrenMut<'a, K, V, const CAPACITY: usize> = (
    &'a mut Node<K, V, CAPACITY>,
    &'a mut K,
    &'a mut Node<K, V, CAPACITY>,
);

impl<K, V, const CAPACITY: usize> Node<K, V, CAPACITY> {
    /// Creates the canonical empty root node.
    pub(crate) fn empty_leaf() -> Self {
        Self::Leaf(LeafNode::empty())
    }

    /// Creates a branch root from a split node.
    pub(crate) fn from_root_split(left: Self, separator: K, right: Box<Self>) -> Self {
        Self::Branch(BranchNode::from_root_split(left, separator, right))
    }

    /// Moves leaf entries into one ordered buffer, discarding branch separators.
    pub(crate) fn append_owned_entries(self, entries: &mut Vec<(K, V)>) {
        match self {
            Self::Leaf(leaf) => leaf.append_owned_entries(entries),
            Self::Branch(branch) => branch.append_owned_entries(entries),
        }
    }

    /// Returns the first leaf entry, if present.
    pub(crate) fn first_key_value(&self) -> Option<(&K, &V)> {
        match self {
            Self::Leaf(leaf) => leaf.first_key_value(),
            Self::Branch(branch) => branch.first_key_value(),
        }
    }

    /// Returns the last leaf entry, if present.
    pub(crate) fn last_key_value(&self) -> Option<(&K, &V)> {
        match self {
            Self::Leaf(leaf) => leaf.last_key_value(),
            Self::Branch(branch) => branch.last_key_value(),
        }
    }

    /// Finds a value by an owned or borrowed key.
    pub(crate) fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self {
            Self::Leaf(leaf) => leaf.get(key),
            Self::Branch(branch) => branch.child_for_key(key).get(key),
        }
    }

    /// Finds a mutable value by an owned or borrowed key.
    pub(crate) fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self {
            Self::Leaf(leaf) => leaf.get_mut(key),
            Self::Branch(branch) => branch.child_for_key_mut(key).get_mut(key),
        }
    }

    /// Inserts into a leaf or recursively into a branch.
    pub(crate) fn insert(&mut self, key: K, value: V) -> InsertResult<K, V, CAPACITY>
    where
        K: Ord + Clone,
    {
        match self {
            Self::Leaf(leaf) => leaf.insert(key, value),
            Self::Branch(branch) => branch.insert(key, value),
        }
    }

    /// Returns the minimum leaf key in this subtree.
    fn minimum_key(&self) -> Option<&K> {
        match self {
            Self::Leaf(leaf) => leaf.entries.first().map(Entry::key),
            Self::Branch(branch) => branch.leftmost.minimum_key(),
        }
    }

    fn is_underfull(&self) -> bool {
        match self {
            Self::Leaf(leaf) => leaf.entries.len() < CAPACITY.div_ceil(2),
            Self::Branch(branch) => branch.child_count() < CAPACITY.saturating_add(1).div_ceil(2),
        }
    }

    fn can_lend(&self) -> bool {
        match self {
            Self::Leaf(leaf) => leaf.entries.len() > CAPACITY.div_ceil(2),
            Self::Branch(branch) => branch.child_count() > CAPACITY.saturating_add(1).div_ceil(2),
        }
    }

    /// Collapses transient one-child branches at the root.
    pub(crate) fn normalize_root(&mut self) {
        loop {
            match self {
                Self::Leaf(_leaf) => return,
                Self::Branch(branch) => {
                    if branch.child_count() != 1 {
                        return;
                    }

                    let child =
                        std::mem::replace(&mut branch.leftmost, Box::new(Self::empty_leaf()));
                    *self = *child;
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn is_empty_leaf(&self) -> bool {
        match self {
            Self::Leaf(leaf) => leaf.is_empty(),
            Self::Branch(_branch) => false,
        }
    }

    #[cfg(test)]
    pub(crate) fn allocated_entry_capacity(&self) -> usize {
        match self {
            Self::Leaf(leaf) => leaf.allocated_entry_capacity(),
            Self::Branch(branch) => branch.allocated_entry_capacity(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_leaf(leaf: LeafNode<K, V, CAPACITY>) -> Self {
        Self::Leaf(leaf)
    }

    #[cfg(test)]
    pub(crate) fn from_branch(branch: BranchNode<K, V, CAPACITY>) -> Self {
        Self::Branch(branch)
    }

    #[cfg(test)]
    pub(crate) fn from_sorted_entries(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        Self::Leaf(LeafNode::from_sorted_entries(entries))
    }

    #[cfg(test)]
    pub(crate) fn from_sorted_branch(
        leftmost: Self,
        first_right: (K, Self),
        remaining: impl IntoIterator<Item = (K, Self)>,
    ) -> Self {
        Self::Branch(BranchNode::from_sorted_parts(
            leftmost,
            first_right,
            remaining,
        ))
    }

    #[cfg(test)]
    pub(crate) fn entry_count(&self) -> usize {
        match self {
            Self::Leaf(leaf) => leaf.entry_count(),
            Self::Branch(branch) => branch.entry_count(),
        }
    }

    #[cfg(test)]
    pub(crate) fn height(&self) -> usize {
        match self {
            Self::Leaf(_leaf) => 0,
            Self::Branch(branch) => branch.height(),
        }
    }
}

impl<K: Ord + Clone, V, const CAPACITY: usize> Node<K, V, CAPACITY> {
    /// Removes from this subtree and reports occupancy changes.
    pub(crate) fn remove<Q>(&mut self, key: &Q) -> RemoveResult<K, V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let previous_minimum = self.minimum_key().cloned();
        let removal = match self {
            Self::Leaf(leaf) => leaf.remove(key),
            Self::Branch(branch) => branch.remove(key),
        };

        match removal {
            Removal::Missing => RemoveResult::Missing,
            Removal::Removed(value) => {
                let minimum = MinimumChange::between(previous_minimum, self.minimum_key());
                let occupancy = if self.is_underfull() {
                    OccupancyChange::Underflow { minimum }
                } else {
                    OccupancyChange::Stable { minimum }
                };
                RemoveResult::Removed { value, occupancy }
            }
        }
    }

    fn borrow_from_left(left: &mut Self, separator: &mut K, right: &mut Self) {
        match (left, right) {
            (Self::Leaf(left), Self::Leaf(right)) => {
                let entry = match left.entries.pop() {
                    Some(entry) => entry,
                    None => unreachable!("a lending leaf contains an entry"),
                };
                *separator = entry.key().clone();
                right.entries.insert(0, entry);
            }
            (Self::Branch(left), Self::Branch(right)) => {
                let edge = match left.rightward.pop() {
                    Some(edge) => edge,
                    None => unreachable!("a lending branch contains a rightward child"),
                };
                let previous_leftmost = std::mem::replace(&mut right.leftmost, edge.child);
                let previous_separator = std::mem::replace(separator, edge.lower_bound);
                right
                    .rightward
                    .insert(0, BranchEdge::new(previous_separator, previous_leftmost));
            }
            (Self::Leaf(_left), Self::Branch(_right)) => {
                unreachable!("balanced tree siblings have matching node shapes")
            }
            (Self::Branch(_left), Self::Leaf(_right)) => {
                unreachable!("balanced tree siblings have matching node shapes")
            }
        }
    }

    fn borrow_from_right(left: &mut Self, separator: &mut K, right: &mut Self) {
        match (left, right) {
            (Self::Leaf(left), Self::Leaf(right)) => {
                let entry = right.entries.remove(0);
                left.entries.push(entry);
                *separator = right.entries[0].key().clone();
            }
            (Self::Branch(left), Self::Branch(right)) => {
                let edge = right.rightward.remove(0);
                let borrowed = std::mem::replace(&mut right.leftmost, edge.child);
                let previous_separator = std::mem::replace(separator, edge.lower_bound);
                left.rightward
                    .push(BranchEdge::new(previous_separator, borrowed));
            }
            (Self::Leaf(_left), Self::Branch(_right)) => {
                unreachable!("balanced tree siblings have matching node shapes")
            }
            (Self::Branch(_left), Self::Leaf(_right)) => {
                unreachable!("balanced tree siblings have matching node shapes")
            }
        }
    }

    fn merge_right(&mut self, separator: K, right: Self) {
        match (self, right) {
            (Self::Leaf(left), Self::Leaf(mut right)) => {
                let _separator = separator;
                left.entries.append(&mut right.entries);
            }
            (Self::Branch(left), Self::Branch(mut right)) => {
                left.rightward
                    .push(BranchEdge::new(separator, right.leftmost));
                left.rightward.append(&mut right.rightward);
            }
            (Self::Leaf(_left), Self::Branch(_right)) => {
                unreachable!("balanced tree siblings have matching node shapes")
            }
            (Self::Branch(_left), Self::Leaf(_right)) => {
                unreachable!("balanced tree siblings have matching node shapes")
            }
        }
    }
}

/// Position returned by a leaf binary search.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SearchSlot {
    Occupied(EntryIndex),
    Vacant(EntryIndex),
}

impl SearchSlot {
    #[cfg(test)]
    const fn occupied(index: usize) -> Self {
        Self::Occupied(EntryIndex::new(index))
    }

    #[cfg(test)]
    const fn vacant(index: usize) -> Self {
        Self::Vacant(EntryIndex::new(index))
    }
}

/// Position in a leaf's entry buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EntryIndex(usize);

impl EntryIndex {
    const fn new(index: usize) -> Self {
        Self(index)
    }

    pub(crate) const fn get(self) -> usize {
        self.0
    }
}

/// Sorted entries owned by a leaf node.
#[derive(Debug, PartialEq, Eq)]
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

    fn append_owned_entries(self, entries: &mut Vec<(K, V)>) {
        entries.extend(self.entries.into_iter().map(Entry::into_pair));
    }

    /// Returns the entry at a cursor position.
    pub(crate) fn entry_at(&self, index: usize) -> Option<(&K, &V)> {
        self.entries.get(index).map(Entry::as_pair)
    }

    /// Returns the number of entries available to a cursor.
    pub(crate) const fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Locates an occupied entry or its insertion position.
    pub(crate) fn search<Q>(&self, key: &Q) -> SearchSlot
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self
            .entries
            .binary_search_by(|entry| entry.key().borrow().cmp(key))
        {
            Ok(index) => SearchSlot::Occupied(EntryIndex::new(index)),
            Err(index) => SearchSlot::Vacant(EntryIndex::new(index)),
        }
    }

    fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.search(key) {
            SearchSlot::Occupied(index) => Some(self.entries[index.get()].value()),
            SearchSlot::Vacant(_insertion_index) => None,
        }
    }

    fn get_mut<Q>(&mut self, key: &Q) -> Option<&mut V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.search(key) {
            SearchSlot::Occupied(index) => Some(self.entries[index.get()].value_mut()),
            SearchSlot::Vacant(_insertion_index) => None,
        }
    }

    /// Inserts an entry, reporting replacement or structural growth.
    pub(crate) fn insert(&mut self, key: K, value: V) -> InsertResult<K, V, CAPACITY>
    where
        K: Ord + Clone,
    {
        match self.search(&key) {
            SearchSlot::Occupied(index) => self.replace_value(index, value),
            SearchSlot::Vacant(index) => self.insert_new(index, key, value),
        }
    }

    fn replace_value(&mut self, index: EntryIndex, value: V) -> InsertResult<K, V, CAPACITY> {
        let previous = std::mem::replace(self.entries[index.get()].value_mut(), value);
        InsertResult::Replaced { previous }
    }

    fn insert_new(&mut self, index: EntryIndex, key: K, value: V) -> InsertResult<K, V, CAPACITY>
    where
        K: Clone,
    {
        self.reserve_overflow_capacity();
        self.entries.insert(index.get(), Entry::new(key, value));

        if self.entries.len() <= CAPACITY {
            InsertResult::Inserted
        } else {
            self.split()
        }
    }

    fn reserve_overflow_capacity(&mut self) {
        let overflow_capacity = CAPACITY.saturating_add(1);
        let additional = overflow_capacity.saturating_sub(self.entries.len());
        self.entries.reserve_exact(additional);
    }

    fn split(&mut self) -> InsertResult<K, V, CAPACITY>
    where
        K: Clone,
    {
        let split_index = CAPACITY.saturating_add(1) / 2;
        let separator = self.entries[split_index].key().clone();
        let mut right_entries = self.entries.split_off(split_index);
        let additional = CAPACITY
            .saturating_add(1)
            .saturating_sub(right_entries.len());
        right_entries.reserve_exact(additional);
        let right = Box::new(Node::Leaf(Self {
            entries: right_entries,
        }));

        InsertResult::InsertedAndSplit { separator, right }
    }

    fn remove<Q>(&mut self, key: &Q) -> Removal<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.search(key) {
            SearchSlot::Occupied(index) => {
                let Entry { value, .. } = self.entries.remove(index.get());
                Removal::Removed(value)
            }
            SearchSlot::Vacant(_insertion_index) => Removal::Missing,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_sorted_entries(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|(key, value)| Entry::new(key, value))
                .collect(),
        }
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

/// Leftmost child plus ordered separator-child edges.
///
/// Stable branches have at least two children. Removal may temporarily leave one
/// child while underflow propagates to the parent or root.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BranchNode<K, V, const CAPACITY: usize> {
    leftmost: Box<Node<K, V, CAPACITY>>,
    rightward: Vec<BranchEdge<K, V, CAPACITY>>,
}

impl<K, V, const CAPACITY: usize> BranchNode<K, V, CAPACITY> {
    fn from_root_split(
        leftmost: Node<K, V, CAPACITY>,
        lower_bound: K,
        right: Box<Node<K, V, CAPACITY>>,
    ) -> Self {
        let mut rightward = Vec::with_capacity(CAPACITY.saturating_add(1));
        rightward.push(BranchEdge::new(lower_bound, right));

        Self {
            leftmost: Box::new(leftmost),
            rightward,
        }
    }

    fn first_key_value(&self) -> Option<(&K, &V)> {
        self.leftmost.first_key_value()
    }

    fn last_key_value(&self) -> Option<(&K, &V)> {
        self.rightmost_child().last_key_value()
    }

    fn append_owned_entries(self, entries: &mut Vec<(K, V)>) {
        self.leftmost.append_owned_entries(entries);
        self.rightward
            .into_iter()
            .for_each(|edge| edge.child.append_owned_entries(entries));
    }

    fn rightmost_child(&self) -> &Node<K, V, CAPACITY> {
        self.rightward
            .last()
            .map_or(self.leftmost.as_ref(), |edge| edge.child.as_ref())
    }

    fn edge_count(&self) -> usize {
        self.rightward.len()
    }

    /// Returns the number of child subtrees available to a cursor.
    pub(crate) fn child_count(&self) -> usize {
        self.edge_count().saturating_add(1)
    }

    fn child_position<Q>(&self, key: &Q) -> ChildIndex
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        ChildIndex::new(
            self.rightward
                .partition_point(|edge| edge.lower_bound.borrow().cmp(key) != Ordering::Greater),
        )
    }

    /// Returns the child index selected for a key.
    pub(crate) fn child_index_for_key<Q>(&self, key: &Q) -> usize
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        self.child_position(key).get()
    }

    /// Returns the child at an ordered cursor position.
    pub(crate) fn child_at(&self, index: usize) -> Option<&Node<K, V, CAPACITY>> {
        match index {
            0 => Some(self.leftmost.as_ref()),
            rightward => self
                .rightward
                .get(rightward.saturating_sub(1))
                .map(|edge| edge.child.as_ref()),
        }
    }

    pub(crate) fn child_for_key<Q>(&self, key: &Q) -> &Node<K, V, CAPACITY>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.child_position(key).get() {
            0 => self.leftmost.as_ref(),
            rightward => self.rightward[rightward - 1].child.as_ref(),
        }
    }

    pub(crate) fn child_for_key_mut<Q>(&mut self, key: &Q) -> &mut Node<K, V, CAPACITY>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let position = self.child_position(key);
        self.child_at_position_mut(position)
    }

    fn child_at_position_mut(&mut self, position: ChildIndex) -> &mut Node<K, V, CAPACITY> {
        match position.get() {
            0 => self.leftmost.as_mut(),
            rightward => self.rightward[rightward - 1].child.as_mut(),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_sorted_parts(
        leftmost: Node<K, V, CAPACITY>,
        first_right: (K, Node<K, V, CAPACITY>),
        remaining: impl IntoIterator<Item = (K, Node<K, V, CAPACITY>)>,
    ) -> Self {
        let (lower_bound, child) = first_right;

        Self {
            leftmost: Box::new(leftmost),
            rightward: std::iter::once(BranchEdge::from_node(lower_bound, child))
                .chain(
                    remaining
                        .into_iter()
                        .map(|(lower_bound, child)| BranchEdge::from_node(lower_bound, child)),
                )
                .collect(),
        }
    }

    #[cfg(test)]
    fn allocated_entry_capacity(&self) -> usize {
        self.leftmost.allocated_entry_capacity()
            + self
                .rightward
                .iter()
                .map(|edge| edge.child.allocated_entry_capacity())
                .sum::<usize>()
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.leftmost.entry_count()
            + self
                .rightward
                .iter()
                .map(|edge| edge.child.entry_count())
                .sum::<usize>()
    }

    #[cfg(test)]
    fn height(&self) -> usize {
        self.leftmost.height().saturating_add(1)
    }
}

impl<K: Ord + Clone, V, const CAPACITY: usize> BranchNode<K, V, CAPACITY> {
    fn insert(&mut self, key: K, value: V) -> InsertResult<K, V, CAPACITY> {
        let position = self.child_position(&key);
        let child_result = self.child_at_position_mut(position).insert(key, value);

        match child_result {
            InsertResult::Inserted => InsertResult::Inserted,
            InsertResult::Replaced { previous } => InsertResult::Replaced { previous },
            InsertResult::InsertedAndSplit { separator, right } => {
                self.absorb_child_split(position, separator, right)
            }
        }
    }

    fn absorb_child_split(
        &mut self,
        position: ChildIndex,
        separator: K,
        right: Box<Node<K, V, CAPACITY>>,
    ) -> InsertResult<K, V, CAPACITY> {
        self.reserve_overflow_capacity();
        self.insert_edge_after(position, BranchEdge::new(separator, right));

        if self.edge_count() <= CAPACITY {
            InsertResult::Inserted
        } else {
            self.split()
        }
    }

    fn reserve_overflow_capacity(&mut self) {
        let overflow_capacity = CAPACITY.saturating_add(1);
        let additional = overflow_capacity.saturating_sub(self.rightward.len());
        self.rightward.reserve_exact(additional);
    }

    fn insert_edge_after(&mut self, position: ChildIndex, edge: BranchEdge<K, V, CAPACITY>) {
        self.rightward.insert(position.get(), edge);
    }

    fn split(&mut self) -> InsertResult<K, V, CAPACITY> {
        let promoted_index = CAPACITY.saturating_add(1) / 2;
        let mut right_edges = self.rightward.split_off(promoted_index.saturating_add(1));
        let promoted = self.rightward.remove(promoted_index);
        let additional = CAPACITY.saturating_add(1).saturating_sub(right_edges.len());
        right_edges.reserve_exact(additional);
        let right = Box::new(Node::Branch(Self {
            leftmost: promoted.child,
            rightward: right_edges,
        }));

        InsertResult::InsertedAndSplit {
            separator: promoted.lower_bound,
            right,
        }
    }

    fn remove<Q>(&mut self, key: &Q) -> Removal<V>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        let child_index = self.child_position(key);
        let child_result = self.child_at_position_mut(child_index).remove(key);

        match child_result {
            RemoveResult::Missing => Removal::Missing,
            RemoveResult::Removed { value, occupancy } => {
                self.absorb_child_removal(child_index, occupancy);
                Removal::Removed(value)
            }
        }
    }

    fn absorb_child_removal(&mut self, child_index: ChildIndex, occupancy: OccupancyChange<K>) {
        match occupancy {
            OccupancyChange::Stable { minimum } => {
                self.apply_stable_minimum(child_index, minimum);
            }
            OccupancyChange::Underflow { minimum } => {
                self.apply_underflow_minimum(child_index, minimum);
                self.rebalance_child(child_index);
                self.refresh_separators();
            }
        }
    }

    fn apply_underflow_minimum(&mut self, child_index: ChildIndex, minimum: MinimumChange<K>) {
        match (child_index.get(), minimum) {
            (0, MinimumChange::Unchanged | MinimumChange::Removed) => {}
            (0, MinimumChange::Changed(changed)) => drop(changed),
            (_rightward, MinimumChange::Unchanged | MinimumChange::Removed) => {}
            (rightward, MinimumChange::Changed(changed)) => {
                self.rightward[rightward - 1].lower_bound = changed;
            }
        }
    }

    fn apply_stable_minimum(&mut self, child_index: ChildIndex, minimum: MinimumChange<K>) {
        match (child_index.get(), minimum) {
            (0, MinimumChange::Unchanged | MinimumChange::Removed) => {}
            (0, MinimumChange::Changed(changed)) => drop(changed),
            (_rightward, MinimumChange::Unchanged) => {}
            (rightward, MinimumChange::Changed(changed)) => {
                self.rightward[rightward - 1].lower_bound = changed;
            }
            (_rightward, MinimumChange::Removed) => self.refresh_separators(),
        }
    }

    fn rebalance_child(&mut self, child_index: ChildIndex) {
        let index = child_index.get();
        let left_can_lend = index
            .checked_sub(1)
            .and_then(|left| self.child_at(left))
            .is_some_and(Node::can_lend);
        let right_can_lend = self
            .child_at(index.saturating_add(1))
            .is_some_and(Node::can_lend);

        if left_can_lend {
            self.borrow_from_left(index - 1);
        } else if right_can_lend {
            self.borrow_from_right(index);
        } else if index > 0 {
            self.merge_children(index - 1);
        } else {
            self.merge_children(0);
        }
    }

    fn borrow_from_left(&mut self, separator_index: usize) {
        let Some((left, separator, right)) = self.adjacent_children_mut(separator_index) else {
            unreachable!("left sibling and separator exist for borrowing");
        };
        Node::borrow_from_left(left, separator, right);
    }

    fn borrow_from_right(&mut self, separator_index: usize) {
        let Some((left, separator, right)) = self.adjacent_children_mut(separator_index) else {
            unreachable!("right sibling and separator exist for borrowing");
        };
        Node::borrow_from_right(left, separator, right);
    }

    fn adjacent_children_mut(
        &mut self,
        separator_index: usize,
    ) -> Option<AdjacentChildrenMut<'_, K, V, CAPACITY>> {
        if separator_index == 0 {
            let edge = self.rightward.get_mut(0)?;
            return Some((
                self.leftmost.as_mut(),
                &mut edge.lower_bound,
                edge.child.as_mut(),
            ));
        }

        let (leftward, rightward) = self.rightward.split_at_mut(separator_index);
        let left = leftward.get_mut(separator_index - 1)?;
        let right = rightward.get_mut(0)?;
        Some((
            left.child.as_mut(),
            &mut right.lower_bound,
            right.child.as_mut(),
        ))
    }

    fn merge_children(&mut self, separator_index: usize) {
        let right = self.rightward.remove(separator_index);
        let left = self.child_at_position_mut(ChildIndex::new(separator_index));
        left.merge_right(right.lower_bound, *right.child);
    }

    fn refresh_separators(&mut self) {
        self.rightward.iter_mut().for_each(|edge| {
            let Some(minimum) = edge.child.minimum_key() else {
                unreachable!("non-leftmost child contains a minimum key");
            };
            edge.lower_bound = minimum.clone();
        });
    }
}

/// Separator paired with the child whose minimum it duplicates.
#[derive(Debug, PartialEq, Eq)]
struct BranchEdge<K, V, const CAPACITY: usize> {
    lower_bound: K,
    child: Box<Node<K, V, CAPACITY>>,
}

impl<K, V, const CAPACITY: usize> BranchEdge<K, V, CAPACITY> {
    fn new(lower_bound: K, child: Box<Node<K, V, CAPACITY>>) -> Self {
        Self { lower_bound, child }
    }

    #[cfg(test)]
    fn from_node(lower_bound: K, child: Node<K, V, CAPACITY>) -> Self {
        Self::new(lower_bound, Box::new(child))
    }
}

/// Ordered child location selected by branch routing.
#[derive(Clone, Copy)]
struct ChildIndex(usize);

impl ChildIndex {
    const fn new(index: usize) -> Self {
        Self(index)
    }

    const fn get(self) -> usize {
        self.0
    }
}

#[cfg(test)]
#[derive(Clone, Copy)]
enum ValidationPosition {
    Root,
    NonRoot,
}

#[cfg(test)]
struct ValidationStats {
    height: usize,
    entry_count: usize,
}

#[cfg(test)]
impl ValidationStats {
    const fn leaf(entry_count: usize) -> Self {
        Self {
            height: 0,
            entry_count,
        }
    }

    fn merge_sibling(self, sibling: Self) -> Self {
        assert_eq!(self.height, sibling.height);
        Self {
            height: self.height,
            entry_count: self.entry_count + sibling.entry_count,
        }
    }

    fn into_parent(self) -> Self {
        Self {
            height: self.height + 1,
            entry_count: self.entry_count,
        }
    }
}

#[cfg(test)]
impl<K: Ord + std::fmt::Debug, V, const CAPACITY: usize> Node<K, V, CAPACITY> {
    pub(crate) fn assert_valid_root(&self) -> usize {
        self.validate(ValidationPosition::Root).entry_count
    }

    fn validate(&self, position: ValidationPosition) -> ValidationStats {
        match self {
            Self::Leaf(leaf) => leaf.validate(position),
            Self::Branch(branch) => branch.validate(position),
        }
    }
}

#[cfg(test)]
impl<K: Ord + std::fmt::Debug, V, const CAPACITY: usize> LeafNode<K, V, CAPACITY> {
    fn validate(&self, position: ValidationPosition) -> ValidationStats {
        assert!(self.entries.len() <= CAPACITY);
        assert!(
            self.entries
                .windows(2)
                .all(|entries| entries[0].key() < entries[1].key())
        );

        match position {
            ValidationPosition::Root => assert!(self.entries.len() <= CAPACITY),
            ValidationPosition::NonRoot => {
                assert!(self.entries.len() >= CAPACITY.div_ceil(2));
            }
        }

        ValidationStats::leaf(self.entries.len())
    }
}

#[cfg(test)]
impl<K: Ord + std::fmt::Debug, V, const CAPACITY: usize> BranchNode<K, V, CAPACITY> {
    fn validate(&self, position: ValidationPosition) -> ValidationStats {
        let edge_count = self.edge_count();
        assert!(edge_count <= CAPACITY);

        match position {
            ValidationPosition::Root => assert!(edge_count >= 1),
            ValidationPosition::NonRoot => {
                let minimum_children = CAPACITY.saturating_add(1).div_ceil(2);
                assert!(edge_count.saturating_add(1) >= minimum_children);
            }
        }

        assert!(self.edges().map(|edge| &edge.lower_bound).is_sorted());
        self.edges().for_each(|edge| {
            assert_eq!(Some(&edge.lower_bound), edge.child.minimum_key());
        });

        self.edges()
            .fold(
                self.leftmost.validate(ValidationPosition::NonRoot),
                |stats, edge| stats.merge_sibling(edge.child.validate(ValidationPosition::NonRoot)),
            )
            .into_parent()
    }

    fn edges(&self) -> impl Iterator<Item = &BranchEdge<K, V, CAPACITY>> {
        self.rightward.iter()
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Entry<K, V> {
    key: K,
    value: V,
}

impl<K, V> Entry<K, V> {
    fn new(key: K, value: V) -> Self {
        Self { key, value }
    }

    fn key(&self) -> &K {
        &self.key
    }

    fn value(&self) -> &V {
        &self.value
    }

    fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    fn as_pair(&self) -> (&K, &V) {
        (&self.key, &self.value)
    }

    fn into_pair(self) -> (K, V) {
        (self.key, self.value)
    }
}

#[cfg(test)]
mod tests {
    use super::BranchNode;
    use super::InsertResult;
    use super::LeafNode;
    use super::MinimumChange;
    use super::Node;
    use super::OccupancyChange;
    use super::RemoveResult;
    use super::SearchSlot;

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct AccountId(u64);

    impl AccountId {
        const fn new(value: u64) -> Self {
            Self(value)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct BalanceCents(u64);

    impl BalanceCents {
        const fn new(value: u64) -> Self {
            Self(value)
        }
    }

    type AccountNode = Node<AccountId, BalanceCents, 3>;

    fn account_leaf<const CAPACITY: usize>(
        account_ids: impl IntoIterator<Item = u64>,
    ) -> LeafNode<AccountId, BalanceCents, CAPACITY> {
        LeafNode::from_sorted_entries(account_ids.into_iter().map(|account_id| {
            (
                AccountId::new(account_id),
                BalanceCents::new(account_id * 10),
            )
        }))
    }

    fn account_branch() -> BranchNode<AccountId, BalanceCents, 3> {
        BranchNode::from_sorted_parts(
            Node::from_leaf(account_leaf([1_001, 1_501])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501])),
            ),
            [(
                AccountId::new(3_001),
                Node::from_leaf(account_leaf([3_001, 3_501])),
            )],
        )
    }

    fn account_branch_node<const CHILDREN: usize>(
        child_accounts: [[u64; 2]; CHILDREN],
    ) -> Node<AccountId, BalanceCents, 3> {
        let mut children = child_accounts.into_iter();
        let leftmost = children
            .next()
            .expect("test branch must contain a leftmost child");
        let first_right = children
            .next()
            .expect("test branch must contain a right child");

        Node::from_sorted_branch(
            Node::from_leaf(account_leaf(leftmost)),
            (
                AccountId::new(first_right[0]),
                Node::from_leaf(account_leaf(first_right)),
            ),
            children.map(|accounts| {
                (
                    AccountId::new(accounts[0]),
                    Node::from_leaf(account_leaf(accounts)),
                )
            }),
        )
    }

    fn underfull_account_branch(accounts: [u64; 2]) -> Node<AccountId, BalanceCents, 3> {
        Node::Branch(BranchNode {
            leftmost: Box::new(Node::from_leaf(account_leaf(accounts))),
            rightward: Vec::new(),
        })
    }

    fn account_ids<const CAPACITY: usize>(
        leaf: &LeafNode<AccountId, BalanceCents, CAPACITY>,
    ) -> Vec<AccountId> {
        leaf.entries.iter().map(|entry| entry.key).collect()
    }

    fn account_entries<const CAPACITY: usize>(
        leaf: &LeafNode<AccountId, BalanceCents, CAPACITY>,
    ) -> Vec<(AccountId, BalanceCents)> {
        leaf.entries
            .iter()
            .map(|entry| (entry.key, entry.value))
            .collect()
    }

    #[test]
    fn search_existing_account_id_returns_occupied_slot() {
        let leaf = account_leaf::<4>([1_001, 2_001, 3_001]);

        assert_eq!(leaf.search(&AccountId::new(2_001)), SearchSlot::occupied(1));
    }

    #[test]
    fn search_first_account_id_returns_first_occupied_slot() {
        let leaf = account_leaf::<4>([1_001, 2_001, 3_001]);

        assert_eq!(leaf.search(&AccountId::new(1_001)), SearchSlot::occupied(0));
    }

    #[test]
    fn search_last_account_id_returns_last_occupied_slot() {
        let leaf = account_leaf::<4>([1_001, 2_001, 3_001]);

        assert_eq!(leaf.search(&AccountId::new(3_001)), SearchSlot::occupied(2));
    }

    #[test]
    fn search_smaller_account_id_returns_first_vacancy() {
        let leaf = account_leaf::<4>([1_001, 2_001, 3_001]);

        assert_eq!(leaf.search(&AccountId::new(901)), SearchSlot::vacant(0));
    }

    #[test]
    fn search_intermediate_account_id_returns_middle_vacancy() {
        let leaf = account_leaf::<4>([1_001, 2_001, 3_001]);

        assert_eq!(leaf.search(&AccountId::new(2_501)), SearchSlot::vacant(2));
    }

    #[test]
    fn search_larger_account_id_returns_last_vacancy() {
        let leaf = account_leaf::<4>([1_001, 2_001, 3_001]);

        assert_eq!(leaf.search(&AccountId::new(4_001)), SearchSlot::vacant(3));
    }

    #[test]
    fn child_for_key_below_first_separator_selects_leftmost() {
        let branch = account_branch();

        assert_eq!(
            branch
                .child_for_key(&AccountId::new(1_750))
                .first_key_value(),
            Some((&AccountId::new(1_001), &BalanceCents::new(10_010)))
        );
    }

    #[test]
    fn child_for_key_equal_to_separator_selects_right_child() {
        let branch = account_branch();

        assert_eq!(
            branch
                .child_for_key(&AccountId::new(2_001))
                .first_key_value(),
            Some((&AccountId::new(2_001), &BalanceCents::new(20_010)))
        );
    }

    #[test]
    fn child_for_key_between_separators_selects_enclosing_child() {
        let branch = account_branch();

        assert_eq!(
            branch
                .child_for_key(&AccountId::new(2_750))
                .first_key_value(),
            Some((&AccountId::new(2_001), &BalanceCents::new(20_010)))
        );
    }

    #[test]
    fn child_for_key_above_last_separator_selects_rightmost() {
        let branch = account_branch();

        assert_eq!(
            branch
                .child_for_key(&AccountId::new(3_750))
                .first_key_value(),
            Some((&AccountId::new(3_001), &BalanceCents::new(30_010)))
        );
    }

    #[test]
    fn child_for_key_mut_below_first_separator_selects_leftmost() {
        let mut branch = account_branch();

        let child = branch.child_for_key_mut(&AccountId::new(1_750));

        assert_eq!(
            child.first_key_value(),
            Some((&AccountId::new(1_001), &BalanceCents::new(10_010)))
        );
    }

    #[test]
    fn child_for_key_mut_between_separators_selects_first_right() {
        let mut branch = account_branch();

        let child = branch.child_for_key_mut(&AccountId::new(2_750));

        assert_eq!(
            child.first_key_value(),
            Some((&AccountId::new(2_001), &BalanceCents::new(20_010)))
        );
    }

    #[test]
    fn child_for_key_mut_above_last_separator_selects_rightmost() {
        let mut branch = account_branch();

        let child = branch.child_for_key_mut(&AccountId::new(3_750));

        assert_eq!(
            child.first_key_value(),
            Some((&AccountId::new(3_001), &BalanceCents::new(30_010)))
        );
    }

    #[test]
    fn branch_node_reports_non_leaf_shape_and_allocated_entries() {
        let node = Node::from_branch(account_branch());

        assert!(!node.is_empty_leaf());
        assert!(node.allocated_entry_capacity() > 0);
    }

    #[test]
    fn insert_into_empty_leaf_adds_account_balance() {
        let mut leaf = account_leaf::<3>([]);

        let result = leaf.insert(AccountId::new(1_001), BalanceCents::new(12_500));

        assert_eq!(result, InsertResult::Inserted);
        assert_eq!(account_ids(&leaf), [AccountId::new(1_001)]);
        assert_eq!(
            leaf.get(&AccountId::new(1_001)),
            Some(&BalanceCents::new(12_500))
        );
    }

    #[test]
    fn insert_new_account_into_non_full_leaf_preserves_order() {
        let mut leaf = account_leaf::<5>([2_001, 4_001]);

        let before = leaf.insert(AccountId::new(1_001), BalanceCents::new(12_500));
        let between = leaf.insert(AccountId::new(3_001), BalanceCents::new(37_500));
        let after = leaf.insert(AccountId::new(5_001), BalanceCents::new(62_500));

        assert_eq!(before, InsertResult::Inserted);
        assert_eq!(between, InsertResult::Inserted);
        assert_eq!(after, InsertResult::Inserted);
        assert_eq!(
            account_ids(&leaf),
            [
                AccountId::new(1_001),
                AccountId::new(2_001),
                AccountId::new(3_001),
                AccountId::new(4_001),
                AccountId::new(5_001),
            ]
        );
    }

    #[test]
    fn insert_existing_account_replaces_balance() {
        let mut leaf = account_leaf::<3>([1_001]);
        let capacity_before = leaf.allocated_entry_capacity();

        let result = leaf.insert(AccountId::new(1_001), BalanceCents::new(15_000));

        assert_eq!(
            result,
            InsertResult::Replaced {
                previous: BalanceCents::new(10_010),
            }
        );
        assert_eq!(leaf.entry_count(), 1);
        assert_eq!(leaf.allocated_entry_capacity(), capacity_before);
        assert_eq!(
            leaf.get(&AccountId::new(1_001)),
            Some(&BalanceCents::new(15_000))
        );
    }

    #[test]
    fn insert_into_full_leaf_splits_at_valid_occupancy() {
        let mut leaf = account_leaf::<3>([1_001, 2_001, 3_001]);

        let result = leaf.insert(AccountId::new(2_501), BalanceCents::new(31_250));

        assert_eq!(
            result,
            InsertResult::InsertedAndSplit {
                separator: AccountId::new(2_501),
                right: Box::new(Node::from_sorted_entries([
                    (AccountId::new(2_501), BalanceCents::new(31_250)),
                    (AccountId::new(3_001), BalanceCents::new(30_010)),
                ])),
            }
        );
        assert_eq!(
            account_ids(&leaf),
            [AccountId::new(1_001), AccountId::new(2_001)]
        );
    }

    #[test]
    fn split_leaf_with_odd_capacity_balances_entries() {
        let mut leaf = account_leaf::<3>([1_001, 2_001, 3_001]);

        let result = leaf.insert(AccountId::new(4_001), BalanceCents::new(50_000));

        assert_eq!(
            result,
            InsertResult::InsertedAndSplit {
                separator: AccountId::new(3_001),
                right: Box::new(Node::from_sorted_entries([
                    (AccountId::new(3_001), BalanceCents::new(30_010)),
                    (AccountId::new(4_001), BalanceCents::new(50_000)),
                ])),
            }
        );
        assert_eq!(leaf.entry_count(), 2);
    }

    #[test]
    fn split_leaf_with_even_capacity_balances_entries() {
        let mut leaf = account_leaf::<4>([1_001, 2_001, 3_001, 4_001]);

        let result = leaf.insert(AccountId::new(2_501), BalanceCents::new(31_250));

        assert_eq!(
            result,
            InsertResult::InsertedAndSplit {
                separator: AccountId::new(2_501),
                right: Box::new(Node::from_sorted_entries([
                    (AccountId::new(2_501), BalanceCents::new(31_250)),
                    (AccountId::new(3_001), BalanceCents::new(30_010)),
                    (AccountId::new(4_001), BalanceCents::new(40_010)),
                ])),
            }
        );
        assert_eq!(leaf.entry_count(), 2);
    }

    #[test]
    fn split_leaf_promotes_right_minimum() {
        let mut leaf = account_leaf::<3>([1_001, 2_001, 3_001]);

        let result = leaf.insert(AccountId::new(3_501), BalanceCents::new(43_750));

        assert_eq!(
            result,
            InsertResult::InsertedAndSplit {
                separator: AccountId::new(3_001),
                right: Box::new(Node::from_sorted_entries([
                    (AccountId::new(3_001), BalanceCents::new(30_010)),
                    (AccountId::new(3_501), BalanceCents::new(43_750)),
                ])),
            }
        );
    }

    #[test]
    fn split_leaf_preserves_every_account_exactly_once() {
        let mut leaf = account_leaf::<4>([1_001, 2_001, 3_001, 4_001]);

        let result = leaf.insert(AccountId::new(2_501), BalanceCents::new(31_250));

        let InsertResult::InsertedAndSplit { right, .. } = result else {
            panic!("full leaf insertion must split");
        };
        let Node::Leaf(right_leaf) = *right else {
            panic!("leaf insertion must produce a leaf sibling");
        };
        let all_entries = account_entries(&leaf)
            .into_iter()
            .chain(account_entries(&right_leaf))
            .collect::<Vec<_>>();
        assert_eq!(
            all_entries,
            [
                (AccountId::new(1_001), BalanceCents::new(10_010)),
                (AccountId::new(2_001), BalanceCents::new(20_010)),
                (AccountId::new(2_501), BalanceCents::new(31_250)),
                (AccountId::new(3_001), BalanceCents::new(30_010)),
                (AccountId::new(4_001), BalanceCents::new(40_010)),
            ]
        );
    }

    #[test]
    fn split_leaf_reserves_one_overflow_slot_in_right_sibling() {
        let mut leaf = account_leaf::<3>([1_001, 2_001, 3_001]);

        let result = leaf.insert(AccountId::new(4_001), BalanceCents::new(50_000));

        let InsertResult::InsertedAndSplit { right, .. } = result else {
            panic!("full leaf insertion must split");
        };
        let Node::Leaf(right_leaf) = *right else {
            panic!("leaf insertion must produce a leaf sibling");
        };
        assert!(leaf.allocated_entry_capacity() >= 4);
        assert!(right_leaf.allocated_entry_capacity() >= 4);
    }

    #[test]
    fn first_leaf_insertion_reserves_one_overflow_slot() {
        let mut leaf = account_leaf::<3>([]);

        let result = leaf.insert(AccountId::new(1_001), BalanceCents::new(12_500));

        assert_eq!(result, InsertResult::Inserted);
        assert!(leaf.allocated_entry_capacity() >= 4);
    }

    #[test]
    fn stable_leaf_insertions_and_replacement_reuse_allocation() {
        let mut leaf = account_leaf::<3>([]);
        let first = leaf.insert(AccountId::new(1_001), BalanceCents::new(12_500));
        let reserved_capacity = leaf.allocated_entry_capacity();

        let second = leaf.insert(AccountId::new(2_001), BalanceCents::new(25_000));
        let replacement = leaf.insert(AccountId::new(1_001), BalanceCents::new(15_000));

        assert_eq!(first, InsertResult::Inserted);
        assert_eq!(second, InsertResult::Inserted);
        assert_eq!(
            replacement,
            InsertResult::Replaced {
                previous: BalanceCents::new(12_500),
            }
        );
        assert_eq!(leaf.allocated_entry_capacity(), reserved_capacity);
    }

    #[test]
    fn branch_absorbs_child_replacement_without_growth() {
        let mut branch = account_branch();
        let entry_count_before = branch.entry_count();

        let result = branch.insert(AccountId::new(2_001), BalanceCents::new(25_500));

        assert_eq!(
            result,
            InsertResult::Replaced {
                previous: BalanceCents::new(20_010),
            }
        );
        assert_eq!(branch.entry_count(), entry_count_before);
        assert_eq!(
            branch
                .child_for_key(&AccountId::new(2_001))
                .get(&AccountId::new(2_001)),
            Some(&BalanceCents::new(25_500))
        );
    }

    #[test]
    fn branch_absorbs_stable_child_insertion() {
        let mut branch = account_branch();

        let result = branch.insert(AccountId::new(1_750), BalanceCents::new(21_875));

        assert_eq!(result, InsertResult::Inserted);
        assert_eq!(branch.entry_count(), 7);
        assert_eq!(
            branch
                .child_for_key(&AccountId::new(1_750))
                .get(&AccountId::new(1_750)),
            Some(&BalanceCents::new(21_875))
        );
    }

    #[test]
    fn branch_absorbs_child_split_when_not_full() {
        let mut branch: BranchNode<AccountId, BalanceCents, 3> = BranchNode::from_sorted_parts(
            Node::from_leaf(account_leaf([1_001, 1_501, 1_751])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501])),
            ),
            [],
        );

        let result = branch.insert(AccountId::new(1_601), BalanceCents::new(20_000));

        assert_eq!(result, InsertResult::Inserted);
        assert_eq!(
            branch,
            BranchNode::from_sorted_parts(
                Node::from_leaf(account_leaf([1_001, 1_501])),
                (
                    AccountId::new(1_601),
                    Node::from_sorted_entries([
                        (AccountId::new(1_601), BalanceCents::new(20_000)),
                        (AccountId::new(1_751), BalanceCents::new(17_510)),
                    ]),
                ),
                [(
                    AccountId::new(2_001),
                    Node::from_leaf(account_leaf([2_001, 2_501])),
                )],
            )
        );
    }

    #[test]
    fn branch_absorbs_first_right_child_split_in_order() {
        let mut branch: BranchNode<AccountId, BalanceCents, 3> = BranchNode::from_sorted_parts(
            Node::from_leaf(account_leaf([1_001, 1_501])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501, 2_751])),
            ),
            [],
        );

        let result = branch.insert(AccountId::new(2_601), BalanceCents::new(32_500));

        assert_eq!(result, InsertResult::Inserted);
        assert_eq!(
            branch,
            BranchNode::from_sorted_parts(
                Node::from_leaf(account_leaf([1_001, 1_501])),
                (
                    AccountId::new(2_001),
                    Node::from_leaf(account_leaf([2_001, 2_501])),
                ),
                [(
                    AccountId::new(2_601),
                    Node::from_sorted_entries([
                        (AccountId::new(2_601), BalanceCents::new(32_500)),
                        (AccountId::new(2_751), BalanceCents::new(27_510)),
                    ]),
                )],
            )
        );
    }

    #[test]
    fn branch_absorbs_remaining_child_split_in_order() {
        let mut branch: BranchNode<AccountId, BalanceCents, 3> = BranchNode::from_sorted_parts(
            Node::from_leaf(account_leaf([1_001, 1_501])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501])),
            ),
            [(
                AccountId::new(3_001),
                Node::from_leaf(account_leaf([3_001, 3_501, 3_751])),
            )],
        );

        let result = branch.insert(AccountId::new(3_601), BalanceCents::new(45_000));

        assert_eq!(result, InsertResult::Inserted);
        assert_eq!(
            branch,
            BranchNode::from_sorted_parts(
                Node::from_leaf(account_leaf([1_001, 1_501])),
                (
                    AccountId::new(2_001),
                    Node::from_leaf(account_leaf([2_001, 2_501])),
                ),
                [
                    (
                        AccountId::new(3_001),
                        Node::from_leaf(account_leaf([3_001, 3_501])),
                    ),
                    (
                        AccountId::new(3_601),
                        Node::from_sorted_entries([
                            (AccountId::new(3_601), BalanceCents::new(45_000)),
                            (AccountId::new(3_751), BalanceCents::new(37_510)),
                        ]),
                    ),
                ],
            )
        );
    }

    #[test]
    fn branch_split_promotes_exactly_one_separator() {
        let mut branch: BranchNode<AccountId, BalanceCents, 3> = BranchNode::from_sorted_parts(
            Node::from_leaf(account_leaf([1_001, 1_101, 1_201])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_101])),
            ),
            [
                (
                    AccountId::new(3_001),
                    Node::from_leaf(account_leaf([3_001, 3_101])),
                ),
                (
                    AccountId::new(4_001),
                    Node::from_leaf(account_leaf([4_001, 4_101])),
                ),
            ],
        );

        let result = branch.insert(AccountId::new(1_151), BalanceCents::new(14_375));

        assert_eq!(
            branch,
            BranchNode::from_sorted_parts(
                Node::from_leaf(account_leaf([1_001, 1_101])),
                (
                    AccountId::new(1_151),
                    Node::from_sorted_entries([
                        (AccountId::new(1_151), BalanceCents::new(14_375)),
                        (AccountId::new(1_201), BalanceCents::new(12_010)),
                    ]),
                ),
                [(
                    AccountId::new(2_001),
                    Node::from_leaf(account_leaf([2_001, 2_101])),
                )],
            )
        );
        assert_eq!(
            result,
            InsertResult::InsertedAndSplit {
                separator: AccountId::new(3_001),
                right: Box::new(Node::from_sorted_branch(
                    Node::from_leaf(account_leaf([3_001, 3_101])),
                    (
                        AccountId::new(4_001),
                        Node::from_leaf(account_leaf([4_001, 4_101])),
                    ),
                    [],
                )),
            }
        );
    }

    #[test]
    fn remove_missing_leaf_key_reports_missing() {
        let mut leaf = Node::from_leaf(account_leaf::<3>([1_001, 2_001, 3_001]));

        let result = leaf.remove(&AccountId::new(2_501));

        assert_eq!(result, RemoveResult::Missing);
        assert_eq!(leaf, Node::from_leaf(account_leaf([1_001, 2_001, 3_001])));
    }

    #[test]
    fn remove_from_occupied_leaf_remains_stable() {
        let mut leaf = Node::from_leaf(account_leaf::<3>([1_001, 2_001, 3_001]));

        let result = leaf.remove(&AccountId::new(2_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(20_010),
                occupancy: OccupancyChange::Stable {
                    minimum: MinimumChange::Unchanged,
                },
            }
        );
        assert_eq!(leaf, Node::from_leaf(account_leaf([1_001, 3_001])));
    }

    #[test]
    fn remove_leaf_minimum_reports_new_minimum() {
        let mut leaf = Node::from_leaf(account_leaf::<3>([1_001, 2_001, 3_001]));

        let result = leaf.remove(&AccountId::new(1_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(10_010),
                occupancy: OccupancyChange::Stable {
                    minimum: MinimumChange::Changed(AccountId::new(2_001)),
                },
            }
        );
    }

    #[test]
    fn remove_from_minimal_leaf_reports_underflow() {
        let mut leaf = Node::from_leaf(account_leaf::<3>([1_001, 2_001]));

        let result = leaf.remove(&AccountId::new(1_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(10_010),
                occupancy: OccupancyChange::Underflow {
                    minimum: MinimumChange::Changed(AccountId::new(2_001)),
                },
            }
        );
    }

    #[test]
    fn remove_last_leaf_key_reports_removed_minimum() {
        let mut leaf = Node::from_leaf(account_leaf::<3>([1_001]));

        let result = leaf.remove(&AccountId::new(1_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(10_010),
                occupancy: OccupancyChange::Underflow {
                    minimum: MinimumChange::Removed,
                },
            }
        );
        assert!(leaf.is_empty_leaf());
    }

    #[test]
    fn rebalance_leaf_borrows_from_left_sibling_first() {
        let mut root: AccountNode = Node::from_sorted_branch(
            Node::from_leaf(account_leaf([1_001, 1_501, 1_751])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501])),
            ),
            [(
                AccountId::new(3_001),
                Node::from_leaf(account_leaf([3_001, 3_501, 3_751])),
            )],
        );

        let result = root.remove(&AccountId::new(2_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(20_010),
                occupancy: OccupancyChange::Stable {
                    minimum: MinimumChange::Unchanged,
                },
            }
        );
        assert_eq!(
            root,
            Node::from_sorted_branch(
                Node::from_leaf(account_leaf([1_001, 1_501])),
                (
                    AccountId::new(1_751),
                    Node::from_leaf(account_leaf([1_751, 2_501])),
                ),
                [(
                    AccountId::new(3_001),
                    Node::from_leaf(account_leaf([3_001, 3_501, 3_751])),
                )],
            )
        );
    }

    #[test]
    fn rebalance_leaf_borrows_from_right_sibling() {
        let mut root: AccountNode = Node::from_sorted_branch(
            Node::from_leaf(account_leaf([1_001, 1_501])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501, 2_751])),
            ),
            [],
        );

        let result = root.remove(&AccountId::new(1_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(10_010),
                occupancy: OccupancyChange::Stable {
                    minimum: MinimumChange::Changed(AccountId::new(1_501)),
                },
            }
        );
        assert_eq!(
            root,
            Node::from_sorted_branch(
                Node::from_leaf(account_leaf([1_501, 2_001])),
                (
                    AccountId::new(2_501),
                    Node::from_leaf(account_leaf([2_501, 2_751])),
                ),
                [],
            )
        );
    }

    #[test]
    fn rebalance_leaf_merges_into_left_sibling() {
        let mut root: AccountNode = Node::from_sorted_branch(
            Node::from_leaf(account_leaf([1_001, 1_501])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501])),
            ),
            [(
                AccountId::new(3_001),
                Node::from_leaf(account_leaf([3_001, 3_501])),
            )],
        );

        let result = root.remove(&AccountId::new(2_001));

        assert!(matches!(
            result,
            RemoveResult::Removed {
                occupancy: OccupancyChange::Stable { .. },
                ..
            }
        ));
        assert_eq!(
            root,
            Node::from_sorted_branch(
                Node::from_leaf(account_leaf([1_001, 1_501, 2_501])),
                (
                    AccountId::new(3_001),
                    Node::from_leaf(account_leaf([3_001, 3_501])),
                ),
                [],
            )
        );
    }

    #[test]
    fn stable_parent_stops_underflow_propagation() {
        let mut parent = Node::from_branch(account_branch());

        let result = parent.remove(&AccountId::new(2_001));

        assert_eq!(
            result,
            RemoveResult::Removed {
                value: BalanceCents::new(20_010),
                occupancy: OccupancyChange::Stable {
                    minimum: MinimumChange::Unchanged,
                },
            }
        );
        assert_eq!(
            parent,
            Node::from_sorted_branch(
                Node::from_leaf(account_leaf([1_001, 1_501, 2_501])),
                (
                    AccountId::new(3_001),
                    Node::from_leaf(account_leaf([3_001, 3_501])),
                ),
                [],
            )
        );
    }

    #[test]
    fn rebalance_leaf_merges_right_sibling() {
        let mut root: AccountNode = Node::from_sorted_branch(
            Node::from_leaf(account_leaf([1_001, 1_501])),
            (
                AccountId::new(2_001),
                Node::from_leaf(account_leaf([2_001, 2_501])),
            ),
            [(
                AccountId::new(3_001),
                Node::from_leaf(account_leaf([3_001, 3_501])),
            )],
        );

        let result = root.remove(&AccountId::new(1_001));

        assert!(matches!(
            result,
            RemoveResult::Removed {
                occupancy: OccupancyChange::Stable { .. },
                ..
            }
        ));
        assert_eq!(
            root,
            Node::from_sorted_branch(
                Node::from_leaf(account_leaf([1_501, 2_001, 2_501])),
                (
                    AccountId::new(3_001),
                    Node::from_leaf(account_leaf([3_001, 3_501])),
                ),
                [],
            )
        );
    }

    #[test]
    fn rebalance_branch_borrows_from_left_sibling() {
        let mut left = account_branch_node([[1_001, 1_501], [2_001, 2_501], [3_001, 3_501]]);
        let mut right = underfull_account_branch([4_001, 4_501]);
        let mut separator = AccountId::new(4_001);

        Node::borrow_from_left(&mut left, &mut separator, &mut right);

        assert_eq!(separator, AccountId::new(3_001));
        assert_eq!(left, account_branch_node([[1_001, 1_501], [2_001, 2_501]]));
        assert_eq!(right, account_branch_node([[3_001, 3_501], [4_001, 4_501]]));
    }

    #[test]
    fn rebalance_branch_borrows_from_right_sibling() {
        let mut left = underfull_account_branch([1_001, 1_501]);
        let mut right = account_branch_node([[2_001, 2_501], [3_001, 3_501], [4_001, 4_501]]);
        let mut separator = AccountId::new(2_001);

        Node::borrow_from_right(&mut left, &mut separator, &mut right);

        assert_eq!(separator, AccountId::new(3_001));
        assert_eq!(left, account_branch_node([[1_001, 1_501], [2_001, 2_501]]));
        assert_eq!(right, account_branch_node([[3_001, 3_501], [4_001, 4_501]]));
    }

    #[test]
    fn rebalance_branch_merge_joins_parent_separator() {
        let mut left = account_branch_node([[1_001, 1_501], [2_001, 2_501]]);
        let right = account_branch_node([[3_001, 3_501], [4_001, 4_501]]);

        left.merge_right(AccountId::new(3_001), right);

        assert_eq!(
            left,
            account_branch_node([
                [1_001, 1_501],
                [2_001, 2_501],
                [3_001, 3_501],
                [4_001, 4_501],
            ])
        );
    }

    #[test]
    fn rebalance_branch_merges_right_sibling() {
        let mut root = BranchNode::from_sorted_parts(
            underfull_account_branch([1_001, 1_501]),
            (
                AccountId::new(2_001),
                account_branch_node([[2_001, 2_501], [3_001, 3_501]]),
            ),
            [],
        );

        root.absorb_child_removal(
            super::ChildIndex::new(0),
            OccupancyChange::Underflow {
                minimum: MinimumChange::Unchanged,
            },
        );
        let expected_merged_branch =
            account_branch_node([[1_001, 1_501], [2_001, 2_501], [3_001, 3_501]]);

        assert_eq!(root.leftmost.as_ref(), &expected_merged_branch);
        assert!(root.rightward.is_empty());
    }

    #[test]
    fn rebalance_branch_applies_changed_minimum_before_borrowing() {
        let mut root = BranchNode::from_sorted_parts(
            account_branch_node([[1_001, 1_501], [2_001, 2_501], [3_001, 3_501]]),
            (
                AccountId::new(4_001),
                underfull_account_branch([4_501, 4_751]),
            ),
            [],
        );

        root.absorb_child_removal(
            super::ChildIndex::new(1),
            OccupancyChange::Underflow {
                minimum: MinimumChange::Changed(AccountId::new(4_501)),
            },
        );

        assert_eq!(
            Node::from_branch(root),
            Node::from_sorted_branch(
                account_branch_node([[1_001, 1_501], [2_001, 2_501]]),
                (
                    AccountId::new(3_001),
                    account_branch_node([[3_001, 3_501], [4_501, 4_751]]),
                ),
                [],
            )
        );
    }
}
