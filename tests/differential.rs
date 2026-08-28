use std::collections::BTreeMap as StandardBTreeMap;

use btree::BTree;
use btree::InsertOutcome;
use btree::RemoveOutcome;
use proptest::collection;
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use proptest::test_runner::TestCaseResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct AccountId(u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BalanceCents(u64);

#[derive(Clone, Copy, Debug)]
enum Operation {
    Insert {
        account_id: AccountId,
        balance: BalanceCents,
    },
    Remove {
        account_id: AccountId,
    },
    Lookup {
        account_id: AccountId,
    },
    Range {
        start: AccountId,
        end: AccountId,
    },
    Iterate,
}

fn account_id() -> impl Strategy<Value = AccountId> {
    (1_001_u16..1_129).prop_map(AccountId)
}

fn balance() -> impl Strategy<Value = BalanceCents> {
    (100_u64..10_000_000).prop_map(BalanceCents)
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        5 => (account_id(), balance()).prop_map(|(account_id, balance)| Operation::Insert {
            account_id,
            balance,
        }),
        4 => account_id().prop_map(|account_id| Operation::Remove { account_id }),
        3 => account_id().prop_map(|account_id| Operation::Lookup { account_id }),
        2 => (account_id(), account_id()).prop_map(|(start, end)| Operation::Range {
            start: start.min(end),
            end: start.max(end),
        }),
        1 => Just(Operation::Iterate),
    ]
}

fn assert_insert_outcome(
    actual: InsertOutcome<BalanceCents>,
    expected: Option<BalanceCents>,
) -> TestCaseResult {
    match (actual, expected) {
        (InsertOutcome::Inserted, None) => Ok(()),
        (
            InsertOutcome::Replaced {
                previous: actual_previous,
            },
            Some(expected_previous),
        ) => {
            prop_assert_eq!(actual_previous, expected_previous);
            Ok(())
        }
        (actual, expected) => Err(TestCaseError::fail(format!(
            "insert outcome differs: {actual:?} versus {expected:?}"
        ))),
    }
}

fn assert_remove_outcome(
    actual: RemoveOutcome<BalanceCents>,
    expected: Option<BalanceCents>,
) -> TestCaseResult {
    match (actual, expected) {
        (RemoveOutcome::Missing, None) => Ok(()),
        (RemoveOutcome::Removed { value: actual }, Some(expected)) => {
            prop_assert_eq!(actual, expected);
            Ok(())
        }
        (actual, expected) => Err(TestCaseError::fail(format!(
            "remove outcome differs: {actual:?} versus {expected:?}"
        ))),
    }
}

fn assert_range_outcome<const CAPACITY: usize>(
    actual: &BTree<AccountId, BalanceCents, CAPACITY>,
    expected: &StandardBTreeMap<AccountId, BalanceCents>,
    start: AccountId,
    end: AccountId,
) -> TestCaseResult {
    let actual_entries = actual
        .range(start..=end)
        .map_err(|error| TestCaseError::fail(format!("valid range returned {error:?}")))?
        .map(|(account_id, balance)| (*account_id, *balance))
        .collect::<Vec<_>>();
    let expected_entries = expected
        .range(start..=end)
        .map(|(account_id, balance)| (*account_id, *balance))
        .collect::<Vec<_>>();

    prop_assert_eq!(actual_entries, expected_entries);
    Ok(())
}

fn assert_iteration_outcome<const CAPACITY: usize>(
    actual: &BTree<AccountId, BalanceCents, CAPACITY>,
    expected: &StandardBTreeMap<AccountId, BalanceCents>,
) -> TestCaseResult {
    prop_assert!(actual.iter().eq(expected.iter()));
    prop_assert!(actual.iter().rev().eq(expected.iter().rev()));
    Ok(())
}

fn apply_operation<const CAPACITY: usize>(
    actual: &mut BTree<AccountId, BalanceCents, CAPACITY>,
    expected: &mut StandardBTreeMap<AccountId, BalanceCents>,
    operation: Operation,
) -> TestCaseResult {
    match operation {
        Operation::Insert {
            account_id,
            balance,
        } => assert_insert_outcome(
            actual.insert(account_id, balance),
            expected.insert(account_id, balance),
        )?,
        Operation::Remove { account_id } => {
            assert_remove_outcome(actual.remove(&account_id), expected.remove(&account_id))?;
        }
        Operation::Lookup { account_id } => {
            prop_assert_eq!(actual.get(&account_id), expected.get(&account_id));
        }
        Operation::Range { start, end } => {
            assert_range_outcome(actual, expected, start, end)?;
        }
        Operation::Iterate => assert_iteration_outcome(actual, expected)?,
    }

    prop_assert_eq!(actual.len(), expected.len());
    prop_assert_eq!(actual.is_empty(), expected.is_empty());
    prop_assert_eq!(actual.first_key_value(), expected.first_key_value());
    prop_assert_eq!(actual.last_key_value(), expected.last_key_value());
    assert_iteration_outcome(actual, expected)
}

fn assert_operations_match_standard<const CAPACITY: usize>(
    operations: &[Operation],
) -> TestCaseResult {
    let mut actual = BTree::<AccountId, BalanceCents, CAPACITY>::new();
    let mut expected = StandardBTreeMap::new();

    operations
        .iter()
        .copied()
        .try_for_each(|operation| apply_operation(&mut actual, &mut expected, operation))
}

fn assert_all_capacities_match_standard(operations: &[Operation]) -> TestCaseResult {
    assert_operations_match_standard::<3>(operations)?;
    assert_operations_match_standard::<4>(operations)?;
    assert_operations_match_standard::<7>(operations)?;
    assert_operations_match_standard::<32>(operations)
}

fn boundary_sequences() -> Vec<Vec<Operation>> {
    let ascending = (1_001_u16..=1_096)
        .map(|account_id| Operation::Insert {
            account_id: AccountId(account_id),
            balance: BalanceCents(u64::from(account_id) * 100),
        })
        .collect::<Vec<_>>();
    let descending = (1_001_u16..=1_096)
        .rev()
        .map(|account_id| Operation::Insert {
            account_id: AccountId(account_id),
            balance: BalanceCents(u64::from(account_id) * 100),
        })
        .collect::<Vec<_>>();
    let replacements = (1_001_u16..=1_064)
        .flat_map(|account_id| {
            [
                Operation::Insert {
                    account_id: AccountId(account_id),
                    balance: BalanceCents(u64::from(account_id) * 100),
                },
                Operation::Insert {
                    account_id: AccountId(account_id),
                    balance: BalanceCents(u64::from(account_id) * 125),
                },
            ]
        })
        .collect::<Vec<_>>();
    let merge_cascade = ascending
        .iter()
        .copied()
        .chain((1_001_u16..=1_096).map(|account_id| Operation::Remove {
            account_id: AccountId(account_id),
        }))
        .collect::<Vec<_>>();

    vec![
        Vec::new(),
        ascending,
        descending,
        replacements,
        merge_cascade,
    ]
}

#[test]
fn boundary_sequences_match_standard_map_at_all_planned_capacities() -> TestCaseResult {
    boundary_sequences()
        .iter()
        .try_for_each(|operations| assert_all_capacities_match_standard(operations))
}

proptest! {
    #[test]
    fn generated_operations_match_standard_map_at_all_planned_capacities(
        operations in collection::vec(operation(), 0..128),
    ) {
        assert_all_capacities_match_standard(&operations)?;
    }
}
