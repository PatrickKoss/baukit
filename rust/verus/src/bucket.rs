//! Twin of `baukit_ratelimit::kernels::bucket::{refill, admit}` and of the
//! entry update in `InMemoryRateLimitStore::check_at`.
//!
//! V-02. For every admission trace, the cost admitted over `[t0, t]` is at
//! most `capacity + rate * (t - t0)`.
use vstd::prelude::*;

verus! {

pub open spec fn min_int(a: int, b: int) -> int {
    if a <= b { a } else { b }
}

pub fn refill(tokens: u64, last_ms: u64, now_ms: u64, rate: u64, capacity: u64) -> (out: u64)
    ensures
        out <= capacity,
        out <= tokens + if now_ms > last_ms { (now_ms - last_ms) * rate } else { 0 },
{
    let elapsed = if now_ms > last_ms { now_ms - last_ms } else { 0 };
    let added = if rate != 0 && elapsed > u64::MAX / rate {
        assert(elapsed * rate > u64::MAX) by (nonlinear_arith)
            requires rate > 0, elapsed > u64::MAX / rate;
        u64::MAX
    } else {
        assert(elapsed * rate <= u64::MAX) by (nonlinear_arith)
            requires rate == 0 || elapsed <= u64::MAX / rate;
        elapsed * rate
    };
    let refilled = if added > u64::MAX - tokens { u64::MAX } else { tokens + added };
    if refilled <= capacity { refilled } else { capacity }
}

pub fn admit(tokens: u64, last_ms: u64, now_ms: u64, rate: u64, capacity: u64, cost: u64) -> (out: (bool, u64, u64))
    ensures
        out.1 <= capacity,
        out.0 ==> out.1 + cost <= tokens + if now_ms > last_ms { (now_ms - last_ms) * rate } else { 0 },
        !out.0 ==> out.1 <= tokens + if now_ms > last_ms { (now_ms - last_ms) * rate } else { 0 },
{
    let refilled = refill(tokens, last_ms, now_ms, rate, capacity);
    if refilled >= cost {
        (true, refilled - cost, 0)
    } else {
        let divisor = if rate >= 1 { rate } else { 1 };
        let missing = cost - refilled;
        assert(divisor == 1 ==> missing % divisor == 0) by (nonlinear_arith)
            requires divisor >= 1;
        assert(divisor > 1 ==> missing / divisor < u64::MAX) by (nonlinear_arith)
            requires divisor >= 1, missing <= u64::MAX;
        let retry = missing / divisor + if missing % divisor == 0 { 0 } else { 1 };
        (false, refilled, retry)
    }
}

/// One `check_at` call: the time it runs at and the scaled cost it asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    pub now_ms: u64,
    pub cost: u64,
}

/// Twin of the bucket entry `check_at` keeps for one key: it starts full at
/// `t0`, and each request admits or refuses and stores `tokens` and
/// `last_ms.max(now_ms)`. Returns the admitted total and the final `last_ms`.
///
/// V-02: the admitted cost is at most `capacity + rate * (t - t0)`, with `t`
/// the latest request time.
pub fn replay(t0: u64, rate: u64, capacity: u64, requests: &Vec<Request>) -> (out: (u128, u64))
    ensures
        out.1 >= t0,
        out.0 <= capacity + rate * (out.1 - t0),
        forall|i: int| 0 <= i < requests.len() ==> #[trigger] requests[i].now_ms <= out.1 || out.1 == t0 && requests[i].now_ms < t0,
{
    let mut tokens = capacity;
    let mut last_ms = t0;
    let mut admitted: u128 = 0;
    let mut k: usize = 0;
    while k < requests.len()
        invariant
            0 <= k <= requests.len(),
            last_ms >= t0,
            tokens <= capacity,
            admitted + tokens <= capacity + rate * (last_ms - t0),
            forall|i: int| 0 <= i < k ==> #[trigger] requests[i].now_ms <= last_ms || last_ms == t0 && requests[i].now_ms < t0,
        decreases requests.len() - k,
    {
        let request = requests[k];
        let (allowed, after, _retry) = admit(tokens, last_ms, request.now_ms, rate, capacity, request.cost);
        let next_last = if request.now_ms >= last_ms { request.now_ms } else { last_ms };
        proof {
            assert(capacity + rate * (last_ms - t0) + if request.now_ms > last_ms { (request.now_ms - last_ms) * rate } else { 0 }
                == capacity + rate * (next_last - t0)) by (nonlinear_arith)
                requires next_last == if request.now_ms >= last_ms { request.now_ms } else { last_ms }, last_ms >= t0;
            assert(capacity + rate * (next_last - t0) <= u64::MAX as int + u64::MAX as int * u64::MAX as int) by (nonlinear_arith)
                requires capacity <= u64::MAX, rate <= u64::MAX, 0 <= next_last - t0 <= u64::MAX;
        }
        if allowed {
            admitted = admitted + request.cost as u128;
        }
        tokens = after;
        last_ms = next_last;
        k = k + 1;
    }
    (admitted, last_ms)
}

} // verus!
