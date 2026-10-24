//! Writer-minted ids (ADR-0026).
//!
//! Every logged row id (claims, hyperedges, episodic events, candidate
//! claims, contradictions, vector chunks) comes from one store-wide
//! generator instead of `MAX(id)+1` against the SQLite projection, so a log
//! line is meaningful without the database that happened to be open when
//! it was written, and two stores' logs can be concatenated without id
//! collisions.
//!
//! Format: `((unix_ms - EPOCH_MS) << 12) | random12`, a 53-bit integer that
//! survives JSON/JavaScript number handling. `EPOCH_MS` is
//! 2024-01-01T00:00:00Z, which keeps 41 bits of milliseconds available
//! (good until 2093). The low 12 bits are random so two processes minting
//! in the same millisecond collide with probability 1/4096 per pair, and the
//! generator is strictly monotonic within a process: when the clock yields a
//! candidate `<= last`, `last + 1` is used instead.

use std::hash::{BuildHasher, RandomState};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// 2024-01-01T00:00:00Z in Unix milliseconds.
pub(crate) const EPOCH_MS: i64 = 1_704_067_200_000;

/// Number of random low bits in an id.
const RANDOM_BITS: u32 = 12;

/// Store-wide id generator. Interior mutability because `Store` methods take
/// `&self`; one instance per open store.
pub(crate) struct IdGen {
    last: AtomicI64,
    salt: RandomState,
}

impl IdGen {
    /// Create a generator whose first id is greater than `seed` (the largest
    /// id found in the existing log tail; `0` for a fresh store).
    pub(crate) fn seeded(seed: i64) -> Self {
        Self {
            last: AtomicI64::new(seed.max(0)),
            salt: RandomState::new(),
        }
    }

    /// Mint the next id. Time-ordered, unique store-wide, strictly
    /// increasing within the process.
    pub(crate) fn mint(&self) -> i64 {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let elapsed = (now_ms - EPOCH_MS).max(0);
        let mut current = self.last.load(Ordering::Relaxed);
        loop {
            let random = (self.salt.hash_one(current) & ((1 << RANDOM_BITS) - 1)) as i64;
            let candidate = (elapsed << RANDOM_BITS) | random;
            let next = candidate.max(current + 1);
            match self.last.compare_exchange_weak(
                current,
                next,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(observed) => current = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_strictly_increasing_and_json_safe() {
        let ids = IdGen::seeded(0);
        let mut previous = 0;
        for _ in 0..10_000 {
            let id = ids.mint();
            assert!(id > previous, "{id} <= {previous}");
            assert!(id < (1_i64 << 53), "{id} exceeds 53 bits");
            previous = id;
        }
    }

    #[test]
    fn seed_is_respected() {
        let seed = (1_i64 << 52) + 7;
        let ids = IdGen::seeded(seed);
        assert_eq!(ids.mint(), seed + 1);
    }

    #[test]
    fn id_encodes_mint_time() {
        let before = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let id = IdGen::seeded(0).mint();
        let after = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let ms = (id >> RANDOM_BITS) + EPOCH_MS;
        assert!(
            (before..=after).contains(&ms),
            "{before} <= {ms} <= {after}"
        );
    }
}
