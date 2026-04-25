//! Stable PlayerId assignment.
//!
//! We need deterministic IDs across re-runs so that the same source data
//! always lands on the same `PlayerId` — otherwise tests of derived data
//! (rosters, trades) become flaky. Approach: hash `(name, dob_or_age, team)`
//! and fold to `u32`. Collisions are vanishingly unlikely at <1k players.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use nba3k_core::PlayerId;

pub fn player_id_from(name: &str, age_or_dob: u32, team_seed: u32) -> PlayerId {
    let mut h = DefaultHasher::new();
    name.to_lowercase().trim().hash(&mut h);
    age_or_dob.hash(&mut h);
    team_seed.hash(&mut h);
    let raw = h.finish();
    // Squash to u32 and reserve 0 for sentinel.
    let v = ((raw ^ (raw >> 32)) as u32).max(1);
    PlayerId(v)
}
