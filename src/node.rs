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
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "branch construction arrives with structural insertion"
        )
    )]
    Branch(BranchNode<K, V, CAPACITY>),
}

/// Result of inserting into a subtree.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "leaf insertion connects to public insertion with root growth"
    )
)]
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

impl<K, V, const CAPACITY: usize> Node<K, V, CAPACITY> {
    /// Creates the canonical empty root node.
    pub(crate) fn empty_leaf() -> Self {
        Self::Leaf(LeafNode::empty())
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

    const fn get(self) -> usize {
        self.0
    }
}

/// Sorted entries owned by a leaf node.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LeafNode<K, V, const CAPACITY: usize> {
    entries: Vec<Entry<K, V>>,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "leaf insertion connects to public insertion with root growth"
    )
)]
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

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

/// Branch with at least two children.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct BranchNode<K, V, const CAPACITY: usize> {
    leftmost: Box<Node<K, V, CAPACITY>>,
    first_right: BranchEdge<K, V, CAPACITY>,
    remaining: Vec<BranchEdge<K, V, CAPACITY>>,
}

impl<K, V, const CAPACITY: usize> BranchNode<K, V, CAPACITY> {
    fn first_key_value(&self) -> Option<(&K, &V)> {
        self.leftmost.first_key_value()
    }

    fn last_key_value(&self) -> Option<(&K, &V)> {
        self.rightmost_child().last_key_value()
    }

    fn rightmost_child(&self) -> &Node<K, V, CAPACITY> {
        self.remaining
            .last()
            .map_or(self.first_right.child.as_ref(), |edge| edge.child.as_ref())
    }

    fn child_position<Q>(&self, key: &Q) -> ChildPosition
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.first_right.lower_bound.borrow().cmp(key) {
            Ordering::Greater => ChildPosition::Leftmost,
            Ordering::Less | Ordering::Equal => {
                let rightward_count = self.remaining.partition_point(|edge| {
                    edge.lower_bound.borrow().cmp(key) != Ordering::Greater
                });

                match rightward_count {
                    0 => ChildPosition::FirstRight,
                    count => ChildPosition::Remaining(EdgeIndex::new(count - 1)),
                }
            }
        }
    }

    pub(crate) fn child_for_key<Q>(&self, key: &Q) -> &Node<K, V, CAPACITY>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.child_position(key) {
            ChildPosition::Leftmost => self.leftmost.as_ref(),
            ChildPosition::FirstRight => self.first_right.child.as_ref(),
            ChildPosition::Remaining(index) => self.remaining[index.get()].child.as_ref(),
        }
    }

    pub(crate) fn child_for_key_mut<Q>(&mut self, key: &Q) -> &mut Node<K, V, CAPACITY>
    where
        K: Borrow<Q>,
        Q: Ord + ?Sized,
    {
        match self.child_position(key) {
            ChildPosition::Leftmost => self.leftmost.as_mut(),
            ChildPosition::FirstRight => self.first_right.child.as_mut(),
            ChildPosition::Remaining(index) => self.remaining[index.get()].child.as_mut(),
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
            first_right: BranchEdge::new(lower_bound, child),
            remaining: remaining
                .into_iter()
                .map(|(lower_bound, child)| BranchEdge::new(lower_bound, child))
                .collect(),
        }
    }

    #[cfg(test)]
    fn allocated_entry_capacity(&self) -> usize {
        self.leftmost.allocated_entry_capacity()
            + self.first_right.child.allocated_entry_capacity()
            + self
                .remaining
                .iter()
                .map(|edge| edge.child.allocated_entry_capacity())
                .sum::<usize>()
    }

    #[cfg(test)]
    fn entry_count(&self) -> usize {
        self.leftmost.entry_count()
            + self.first_right.child.entry_count()
            + self
                .remaining
                .iter()
                .map(|edge| edge.child.entry_count())
                .sum::<usize>()
    }
}

/// Separator paired with the child whose minimum it duplicates.
#[derive(Debug, PartialEq, Eq)]
struct BranchEdge<K, V, const CAPACITY: usize> {
    lower_bound: K,
    child: Box<Node<K, V, CAPACITY>>,
}

impl<K, V, const CAPACITY: usize> BranchEdge<K, V, CAPACITY> {
    #[cfg(test)]
    fn new(lower_bound: K, child: Node<K, V, CAPACITY>) -> Self {
        Self {
            lower_bound,
            child: Box::new(child),
        }
    }
}

/// Child location selected by branch routing.
#[derive(Clone, Copy)]
enum ChildPosition {
    Leftmost,
    FirstRight,
    Remaining(EdgeIndex),
}

/// Position in the branch edges after the first right child.
#[derive(Clone, Copy)]
struct EdgeIndex(usize);

impl EdgeIndex {
    const fn new(index: usize) -> Self {
        Self(index)
    }

    const fn get(self) -> usize {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Entry<K, V> {
    key: K,
    value: V,
}

impl<K, V> Entry<K, V> {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "entry construction connects to public insertion with root growth"
        )
    )]
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
}

#[cfg(test)]
mod tests {
    use super::{BranchNode, InsertResult, LeafNode, Node, SearchSlot};

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
}
