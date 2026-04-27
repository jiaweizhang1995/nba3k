//! Retirement engine — decides whether an aging player hangs it up at the
//! end of a season. Pure function; the caller (season-advance) walks the
//! active roster and persists the resulting flag.
//!
//! Rules (M11 charter):
//! - Hard retire if age >= 41.
//! - Conditional retire if age >= 36 AND (overall < 70 OR mins_played < 800).
//! - Stochastic retire at age 39 or 40: deterministic per-player, ~50%
//!   threshold derived from a hash of the player id. No RNG state required —
//!   re-running the pass on the same DB yields the same outcome.

use nba3k_core::Player;

/// Decide whether `player` should retire after a season in which they
/// logged `mins_played` regular-season minutes.
pub fn should_retire(player: &Player, mins_played: u32) -> bool {
    let age = player.age;

    if age >= 41 {
        return true;
    }

    if age >= 36 && (player.overall < 70 || mins_played < 800) {
        return true;
    }

    if age == 39 || age == 40 {
        return stochastic_retire(player.id.0);
    }

    false
}

/// Deterministic ~50% gate keyed on player id. Same id always yields the
/// same answer so the season-advance pass is reproducible across runs.
fn stochastic_retire(player_id: u32) -> bool {
    // FNV-1a 32-bit on the id bytes — cheap, no_std-friendly, well-mixed.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in player_id.to_le_bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash % 2 == 0
}
