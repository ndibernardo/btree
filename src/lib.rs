#![doc = include_str!("../README.md")]

mod iter;
mod node;
mod tree;

pub use iter::Iter;
pub use iter::Range;
pub use iter::RangeError;
pub use tree::BTree;
pub use tree::InsertOutcome;
pub use tree::RemoveOutcome;
