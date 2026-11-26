use baukit_sync::pull::{PullCursor, PullWindow, merge_pull_page};
use baukit_verus::pull as twin;
use std::num::NonZeroUsize;
use uuid::Uuid;

type Pos = (i64, u128);

fn cursor(position: Pos) -> PullCursor {
    PullCursor::after_row(position.0, Uuid::from_u128(position.1))
}

fn same_rows(left: &[Pos], right: &[Pos]) -> bool {
    left.len() == right.len()
        && left.iter().all(|row| {
            left.iter().filter(|other| *other == row).count()
                == right.iter().filter(|other| *other == row).count()
        })
}

/// Merges the same tables with both kernels and compares cursor, `more`,
/// and the rows as a multiset.
#[must_use]
pub fn merge_matches(hi: i64, after: Pos, tables: &[Vec<Pos>], limit: NonZeroUsize) -> bool {
    let production = merge_pull_page(
        &PullWindow {
            hi,
            after: cursor(after),
        },
        tables.to_vec(),
        limit,
        |&position| cursor(position),
    );
    let twin = twin::merge_pull_page(
        &twin::PullWindow { hi, after },
        &tables.to_vec(),
        limit.get(),
    );
    production.more == twin.more
        && production.next == cursor(twin.next)
        && same_rows(&production.rows, &twin.rows)
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
        // No Kani twin here: CBMC runs out of 8 GB on the production Uuid memcmp and
        // sort even for two one-row tables, so this test carries the equivalence.
        #![proptest_config(ProptestConfig::with_cases(4_096))]

        #[test]
        fn merge_matches_production(
            hi in 0_i64..6,
            after in position(),
            tables in vec(table(), 0..4),
            limit in 1_usize..5,
        ) {
            let limit = NonZeroUsize::new(limit).ok_or_else(|| TestCaseError::fail("zero limit"))?;
            prop_assert!(merge_matches(hi, after, &tables, limit));
        }
    }
}
