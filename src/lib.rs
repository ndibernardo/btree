//! Ordered in-memory map backed by a B+-tree.
//!
//! The public API uses the conventional `BTree` name. Values are stored only in
//! leaves.
//!
//! Node capacity is part of the tree's type and must be at least three:
//!
//! ```compile_fail,E0080
//! use btree::BTree;
//!
//! let _invalid = BTree::<u64, u64, 2>::new();
//! ```

mod node;
mod tree;

pub use tree::BTree;
