//! Twin of `baukit_sync::pull::merge_pull_page` with `PullCursor` as a
//! `(revision, id)` pair and `Uuid` as its big-endian `u128`, which orders the
//! same way. The twin keeps the merged rows in table order; production sorts
//! them afterwards, which changes order but not membership.
//!
//! V-03. Every page either advances the cursor strictly or returns
//! `more = false`, and no row with `(revision, id)` above the cursor and at or
//! below `hi` is skipped.
use vstd::prelude::*;

verus! {

pub type Pos = (i64, u128);

pub open spec fn lt(a: Pos, b: Pos) -> bool {
    a.0 < b.0 || (a.0 == b.0 && a.1 < b.1)
}

pub open spec fn le(a: Pos, b: Pos) -> bool {
    lt(a, b) || a == b
}

/// `PullCursor::after_revision`: past every row stamped at or below `revision`.
pub open spec fn after_revision(revision: i64) -> Pos {
    (revision, u128::MAX)
}

pub struct PullWindow {
    pub hi: i64,
    pub after: Pos,
}

pub struct MergedPullPage {
    pub rows: Vec<Pos>,
    pub next: Pos,
    pub more: bool,
}

pub open spec fn full(rows: Seq<Pos>, limit: usize) -> bool {
    rows.len() >= limit && rows.len() > 0
}

/// What `merge_pull_page` returns, stated over its inputs.
pub open spec fn is_merge(window: PullWindow, tables: Seq<Seq<Pos>>, limit: usize, page: MergedPullPage) -> bool {
    &&& page.more <==> exists|i: int| 0 <= i < tables.len() && full(#[trigger] tables[i], limit)
    &&& page.more ==> exists|i: int| 0 <= i < tables.len() && full(#[trigger] tables[i], limit)
            && page.next == tables[i].last()
    &&& page.more ==> forall|i: int| 0 <= i < tables.len() && full(#[trigger] tables[i], limit)
            ==> le(page.next, tables[i].last())
    &&& !page.more ==> page.next == after_revision(window.hi)
    &&& forall|i: int, j: int| 0 <= i < tables.len() && 0 <= j < tables[i].len()
            && (!page.more || le(tables[i][j], page.next)) ==> page.rows@.contains(#[trigger] tables[i][j])
}

/// One table's rows as the `WHERE ... (revision, id) > after ORDER BY revision, id LIMIT limit`
/// query in `pull.rs` returns them from `all`: at most `limit` rows, all inside
/// `(after, hi]`, and it leaves rows of the window out only when it filled its
/// limit, and then only rows above its last row.
pub open spec fn table_read(all: Seq<Pos>, rows: Seq<Pos>, window: PullWindow, limit: usize) -> bool {
    &&& rows.len() <= limit
    &&& forall|j: int| 0 <= j < rows.len() ==> all.contains(#[trigger] rows[j])
            && lt(window.after, rows[j]) && le(rows[j], after_revision(window.hi))
    &&& forall|k: int| 0 <= k < all.len() && lt(window.after, #[trigger] all[k]) && le(all[k], after_revision(window.hi))
            ==> rows.contains(all[k]) || (rows.len() >= limit && (rows.len() == 0 || lt(rows.last(), all[k])))
}

fn pos_le(a: Pos, b: Pos) -> (out: bool)
    ensures out == le(a, b),
{
    a.0 < b.0 || (a.0 == b.0 && a.1 <= b.1)
}

proof fn prefix_keeps_rows(before: Seq<Pos>, after: Seq<Pos>)
    requires before.len() <= after.len(), after.subrange(0, before.len() as int) == before,
    ensures forall|x: Pos| before.contains(x) ==> after.contains(x),
{
    assert forall|x: Pos| before.contains(x) implies after.contains(x) by {
        let w = choose|w: int| 0 <= w < before.len() && before[w] == x;
        assert(after.subrange(0, before.len() as int)[w] == after[w]);
    }
}

pub fn merge_pull_page(window: &PullWindow, tables: &Vec<Vec<Pos>>, limit: usize) -> (page: MergedPullPage)
    ensures is_merge(*window, tables@.map_values(|t: Vec<Pos>| t@), limit, page),
{
    let ghost views = tables@.map_values(|t: Vec<Pos>| t@);
    let mut cut: Option<Pos> = None;
    let mut i: usize = 0;
    while i < tables.len()
        invariant
            0 <= i <= tables.len(),
            views == tables@.map_values(|t: Vec<Pos>| t@),
            cut is Some <==> exists|k: int| 0 <= k < i && full(#[trigger] views[k], limit),
            cut matches Some(c) ==> exists|k: int| 0 <= k < i && full(#[trigger] views[k], limit) && c == views[k].last(),
            cut matches Some(c) ==> forall|k: int| 0 <= k < i && full(#[trigger] views[k], limit) ==> le(c, views[k].last()),
        decreases tables.len() - i,
    {
        let rows = &tables[i];
        assert(views[i as int] == rows@);
        if rows.len() >= limit && rows.len() > 0 {
            let last = rows[rows.len() - 1];
            cut = match cut {
                Some(c) => if pos_le(c, last) { Some(c) } else { Some(last) },
                None => Some(last),
            };
        }
        i = i + 1;
    }
    let mut merged: Vec<Pos> = Vec::new();
    let mut i: usize = 0;
    while i < tables.len()
        invariant
            0 <= i <= tables.len(),
            views == tables@.map_values(|t: Vec<Pos>| t@),
            forall|a: int, j: int| 0 <= a < i && 0 <= j < views[a].len()
                && (cut is None || le(views[a][j], cut->Some_0)) ==> merged@.contains(#[trigger] views[a][j]),
        decreases tables.len() - i,
    {
        let rows = &tables[i];
        assert(views[i as int] == rows@);
        let ghost before = merged@;
        let mut j: usize = 0;
        while j < rows.len()
            invariant
                0 <= j <= rows.len(),
                views[i as int] == rows@,
                before.len() <= merged@.len(),
                merged@.subrange(0, before.len() as int) == before,
                forall|b: int| 0 <= b < j && (cut is None || le(rows@[b], cut->Some_0)) ==> merged@.contains(#[trigger] rows@[b]),
            decreases rows.len() - j,
        {
            let row = rows[j];
            let ghost previous = merged@;
            let keep = match cut {
                Some(c) => pos_le(row, c),
                None => true,
            };
            if keep {
                merged.push(row);
            }
            proof {
                assert(merged@.subrange(0, before.len() as int) =~= before);
                assert forall|b: int| 0 <= b < j + 1 && (cut is None || le(rows@[b], cut->Some_0))
                    implies merged@.contains(#[trigger] rows@[b]) by {
                    if b < j {
                        let w = choose|w: int| 0 <= w < previous.len() && previous[w] == rows@[b];
                        assert(merged@[w] == previous[w]);
                    } else {
                        assert(merged@[merged@.len() - 1] == row);
                    }
                }
            }
            j = j + 1;
        }
        proof {
            prefix_keeps_rows(before, merged@);
        }
        i = i + 1;
    }
    match cut {
        Some(next) => MergedPullPage { rows: merged, next, more: true },
        None => MergedPullPage { rows: merged, next: (window.hi, u128::MAX), more: false },
    }
}

/// V-03 over one page. Given per-table reads of the window that respect
/// `LIMIT limit` with `limit >= 1`: the page advances the cursor strictly or
/// reports `more = false`, and every row of every table inside the window is
/// either in the page or above the returned cursor.
pub proof fn page_progress_and_no_skip(
    window: PullWindow,
    all: Seq<Seq<Pos>>,
    tables: Seq<Seq<Pos>>,
    limit: usize,
    page: MergedPullPage,
)
    requires
        limit >= 1,
        all.len() == tables.len(),
        forall|i: int| 0 <= i < tables.len() ==> table_read(#[trigger] all[i], tables[i], window, limit),
        is_merge(window, tables, limit, page),
    ensures
        page.more ==> lt(window.after, page.next),
        !page.more ==> page.next == after_revision(window.hi),
        forall|i: int, k: int| 0 <= i < all.len() && 0 <= k < all[i].len()
            && lt(window.after, #[trigger] all[i][k]) && le(all[i][k], after_revision(window.hi))
            ==> page.rows@.contains(all[i][k]) || lt(page.next, all[i][k]),
{
    if page.more {
        let i = choose|i: int| 0 <= i < tables.len() && full(#[trigger] tables[i], limit) && page.next == tables[i].last();
        assert(table_read(all[i], tables[i], window, limit));
        assert(lt(window.after, tables[i][tables[i].len() - 1]));
    }
    assert forall|i: int, k: int| 0 <= i < all.len() && 0 <= k < all[i].len()
        && lt(window.after, #[trigger] all[i][k]) && le(all[i][k], after_revision(window.hi))
        implies page.rows@.contains(all[i][k]) || lt(page.next, all[i][k]) by {
        let r = all[i][k];
        assert(table_read(all[i], tables[i], window, limit));
        if tables[i].contains(r) {
            let j = choose|j: int| 0 <= j < tables[i].len() && tables[i][j] == r;
            if !page.more || le(r, page.next) {
                assert(page.rows@.contains(tables[i][j]));
            }
        } else {
            assert(tables[i].len() >= limit);
            assert(full(tables[i], limit));
            assert(le(page.next, tables[i].last()));
        }
    }
}

} // verus!
