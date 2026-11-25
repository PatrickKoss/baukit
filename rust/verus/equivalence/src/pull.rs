use baukit_sync::pull::{PullCursor, PullWindow, merge_pull_page};
use baukit_verus::pull as twin;
use uuid::Uuid;

type Pos = (i64, u128);

fn cursor(position: Pos) -> PullCursor {
    PullCursor::after_row(position.0, Uuid::from_u128(position.1))
}

fn sorted(mut rows: Vec<Pos>) -> Vec<Pos> {
    rows.sort_unstable();
    rows
}

/// Merges the same tables with both kernels and compares cursor, `more`,
/// and the rows as a sorted multiset.
#[must_use]
pub fn merge_matches(hi: i64, after: Pos, tables: &[Vec<Pos>], limit: usize) -> bool {
    let production = merge_pull_page(
        &PullWindow {
            hi,
            after: cursor(after),
        },
        tables.to_vec(),
        limit,
        |&position| cursor(position),
    );
    let twin = twin::merge_pull_page(&twin::PullWindow { hi, after }, &tables.to_vec(), limit);
    production.more == twin.more
        && production.next == cursor(twin.next)
        && production.rows == sorted(twin.rows)
}

#[cfg(test)]
mod tests {
    use proptest::{collection::vec, prelude::*};

    use super::*;

    fn position() -> impl Strategy<Value = Pos> {
        (0_i64..6, prop_oneof![0_u128..4, Just(u128::MAX)])
    }

    fn table() -> impl Strategy<Value = Vec<Pos>> {
        vec(position(), 0..5).prop_map(|mut rows| {
            rows.sort_unstable();
            rows.dedup();
            rows
        })
    }

    proptest! {
        #[test]
        fn merge_matches_production(
            hi in 0_i64..6,
            after in position(),
            tables in vec(table(), 0..4),
            limit in 0_usize..5,
        ) {
            prop_assert!(merge_matches(hi, after, &tables, limit));
        }
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    const TABLES: usize = 2;
    const ROWS: usize = 2;

    fn any_table() -> Vec<Pos> {
        let len: usize = kani::any();
        kani::assume(len <= ROWS);
        let mut rows = Vec::with_capacity(len);
        for _ in 0..len {
            let row: Pos = (kani::any(), kani::any());
            if let Some(&last) = rows.last() {
                kani::assume(last < row);
            }
            rows.push(row);
        }
        rows
    }

    /// Twin and production `merge_pull_page` agree on sorted tables.
    /// Bounds: 2 tables of at most 2 rows, limit <= 3, unwind 6.
    #[kani::proof]
    #[kani::unwind(6)]
    fn merge_equivalent() {
        let tables: Vec<Vec<Pos>> = (0..TABLES).map(|_| any_table()).collect();
        let limit: usize = kani::any();
        kani::assume(limit <= ROWS + 1);
        assert!(merge_matches(
            kani::any(),
            (kani::any(), kani::any()),
            &tables,
            limit
        ));
    }
}
