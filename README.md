# btree

An ordered in-memory map implemented as a B+-tree in Rust.

## Overview

This library stores key-value pairs in sorted order. Values live only in leaf
nodes; branch nodes contain routing separators. The tree supports borrowed-key
lookup, mutable lookup, insertion, removal, double-ended iteration, and bounded
ranges.

Node capacity is part of the tree's type. It defaults to 32 and can be selected
with the `CAPACITY` const generic. The crate has no runtime dependencies.

## Usage

Insert balances, query a bounded range, and remove an entry:

```rust
use std::ops::Bound;

use btree::{BTree, InsertOutcome, RemoveOutcome};

let mut balances = BTree::<String, u64, 3>::new();
assert_eq!(
    balances.insert(String::from("account-2026-1001"), 12_500),
    InsertOutcome::Inserted,
);
assert_eq!(
    balances.insert(String::from("account-2026-2001"), 25_000),
    InsertOutcome::Inserted,
);

assert_eq!(balances.get("account-2026-1001"), Some(&12_500));

let selected = balances
    .range::<str, _>((
        Bound::Included("account-2026-1001"),
        Bound::Excluded("account-2026-3001"),
    ))
    .map(|entries| {
        entries
            .map(|(account_id, balance)| (account_id.as_str(), *balance))
            .collect::<Vec<_>>()
    });

assert_eq!(
    selected,
    Ok(vec![
        ("account-2026-1001", 12_500),
        ("account-2026-2001", 25_000),
    ]),
);

assert_eq!(
    balances.remove("account-2026-1001"),
    RemoveOutcome::Removed { value: 12_500 },
);
```

Insertion reports whether it added a key or replaced an existing value. Removal
reports the removed value or `RemoveOutcome::Missing`.

## Range behavior

`BTree::range` validates its bounds and returns a `Result`:

- a start bound after the end returns `RangeError::StartAfterEnd`;
- equal included bounds describe one possible key;
- equal bounds with either side excluded return
  `RangeError::EmptyExcludedBounds`.

This equal-bound behavior is an explicit crate contract and differs from some
`std::collections::BTreeMap::range` edge cases.

## Node capacity and representation

`CAPACITY` is the maximum number of entries in a stable leaf or separators in a
stable branch. It must be at least three:

```compile_fail,E0080
use btree::BTree;

let _invalid = BTree::<u64, u64, 2>::new();
```

The tree maintains these invariants:

- leaf keys are strictly ordered and unique;
- all leaves have the same depth;
- each branch separator equals the minimum key of its right child;
- non-root nodes satisfy their minimum occupancy;
- the empty tree is represented by one empty root leaf;
- stable branch roots have at least two children.

Insertion and removal require `K: Clone` because branch separators duplicate the
minimum key stored in their right subtree. Lookup and traversal do not clone keys.

## Testing

```bash
cargo test
```

Unit tests cover search, routing, splitting, rebalancing, root normalization,
iteration, and range traversal. Property tests compare generated operation
sequences with `std::collections::BTreeMap` across odd, even, minimum, and default
node capacities. Generated range bounds are checked in both traversal directions.

The complete local verification commands are:

```bash
cargo test --all-targets
cargo test --doc
cargo clippy --all-targets -- -D warnings
cargo fmt --check
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

The repository targets Rust 1.98 and edition 2024. `nix develop` provides an
optional development shell.
