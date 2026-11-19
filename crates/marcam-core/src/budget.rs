//! A work budget for planning one operation: the work of its surface queries (the triangles
//! a drop-cutter examines, the profile points a radial drop reads) and the moves it keeps are
//! counted, and past either limit the queries return at once, no more moves are kept and the
//! strategies' loops stop, so an operation set far too fine (a 0.01 mm stepover over a large
//! part, thousands of levels) ends quickly with an error that says so, instead of running
//! for minutes and filling the memory. Outside planning (no budget started) nothing is
//! limited. The counts are the same on every machine, so the outcome is too.

use std::cell::Cell;

/// Query work one operation may use (triangles examined, profile points read, and
/// [`PER_QUERY`] a query): half a minute natively at the most, about a minute in the browser.
/// The largest real operations use about 50 million.
pub const OP_LIMIT: u64 = 300_000_000;
/// The work a query counts besides the triangles or points it reads.
pub const PER_QUERY: u64 = 16;
/// Moves one operation may keep (each takes about 50 bytes: 150 MB); real ones keep tens of
/// thousands.
pub const MOVE_LIMIT: u64 = 3_000_000;

thread_local! {
    static USED: Cell<u64> = const { Cell::new(0) };
    static LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
    static MOVES: Cell<u64> = const { Cell::new(0) };
    static MOVES_LIMIT: Cell<u64> = const { Cell::new(u64::MAX) };
}

/// Start counting against `limit` units of query work and [`MOVE_LIMIT`] moves (on this
/// thread).
pub fn start(limit: u64) {
    USED.with(|u| u.set(0));
    LIMIT.with(|l| l.set(limit));
    MOVES.with(|u| u.set(0));
    MOVES_LIMIT.with(|l| l.set(MOVE_LIMIT));
}

/// Stop counting (no limit); returns the query work and the moves used.
pub fn stop() -> (u64, u64) {
    LIMIT.with(|l| l.set(u64::MAX));
    MOVES_LIMIT.with(|l| l.set(u64::MAX));
    (USED.with(|u| u.get()), MOVES.with(|u| u.get()))
}

/// Count one move; false once [`MOVE_LIMIT`] is passed (the move is then not kept).
#[inline]
pub fn keep_move() -> bool {
    MOVES.with(|u| {
        let v = u.get() + 1;
        u.set(v);
        v <= MOVES_LIMIT.with(|l| l.get())
    })
}

/// Counting while it lives ([`start`] now, [`stop`] when dropped, whatever the way out).
pub struct Guard;

impl Guard {
    pub fn start(limit: u64) -> Guard {
        start(limit);
        Guard
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        stop();
    }
}

/// Count `n` units; false once the limit is passed (the caller then skips the work).
#[inline]
pub fn spend(n: u64) -> bool {
    USED.with(|u| {
        let v = u.get().saturating_add(n);
        u.set(v);
        v <= LIMIT.with(|l| l.get())
    })
}

/// Whether a limit has been passed (loops stop early).
#[inline]
pub fn over() -> bool {
    USED.with(|u| u.get()) > LIMIT.with(|l| l.get())
        || MOVES.with(|u| u.get()) > MOVES_LIMIT.with(|l| l.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_to_the_limit_then_refuses() {
        start(3);
        assert!(spend(1) && spend(2) && !over());
        assert!(!spend(1) && over());
        assert_eq!(stop(), (4, 0));
        assert!(spend(1_000_000) && !over(), "no limit outside planning");
    }
}
