//! Worker B — star protection / untouchable model.
//!
//! Returns a `Score` with `value` in `[0.0, 1.0]`. 1.0 = absolute
//! untouchable (Luka on LAL, Jokic on DEN). The trade engine reads
//! this BEFORE running value math:
//!   ≥ 0.85 → reject any offer regardless of value.
//!   0.60-0.85 → require value premium.
//!   < 0.60 → no extra friction.
//!
//! Sources of protection (each emits a Reason):
//!   - franchise_tag      — name listed in `data/star_roster.toml` for the
//!                          owning team. Hard-locks at `franchise_tag_value`
//!                          (default 1.0) so even an "underrated" Luka pings
//!                          untouchable.
//!   - top_ovr_on_team    — highest-OVR player on the team gets a bump.
//!                          Folds in a Contend-mode amplifier so contenders
//!                          guard their #1 harder than rebuilders do.
//!   - young_ascending    — age ≤ 24 + potential ≥ 90 (the next-Luka prospect).
//!   - recent_signing     — signed/extended in last 12 sim months. Skipped
//!                          when contract.signed_in_season is missing.
//!
//! See `phases/M4-realism.md` "Worker B" for the full spec.

use crate::team_context::{team_context, TeamMode};
use crate::weights::{StarProtectionWeights, TeamContextWeights};
use crate::Score;
use nba3k_core::{LeagueSnapshot, PlayerId, TeamId};
use serde::Deserialize;

/// Roster file path read by the v1 implementation. Worker B may relocate.
pub const STAR_ROSTER_PATH: &str = "data/star_roster.toml";

#[derive(Debug, Default, Clone)]
pub struct StarRoster {
    /// abbrev (uppercased) → list of player names tagged as franchise.
    /// Names are stored case-preserved; matching is case-insensitive.
    pub by_team: std::collections::HashMap<String, Vec<String>>,
}

impl StarRoster {
    /// True iff `player_name` is tagged for `team_abbrev`. Both keys matched
    /// case-insensitively.
    pub fn is_tagged(&self, team_abbrev: &str, player_name: &str) -> bool {
        let key = team_abbrev.to_ascii_uppercase();
        let Some(names) = self.by_team.get(&key) else {
            return false;
        };
        names.iter().any(|n| n.eq_ignore_ascii_case(player_name))
    }

    /// Total number of teams with at least one tagged player.
    pub fn team_count(&self) -> usize {
        self.by_team.values().filter(|v| !v.is_empty()).count()
    }
}

/// Wire schema for `data/star_roster.toml`. Each top-level table is a team
/// abbrev (e.g. `[BOS]`) with a `players = [...]` list.
#[derive(Debug, Deserialize)]
struct TeamEntry {
    #[serde(default)]
    players: Vec<String>,
}

pub fn load_star_roster(path: &std::path::Path) -> crate::ModelResult<StarRoster> {
    let text = match std::fs::read_to_string(path) {
        Ok(s) => s,
        // Missing file is NOT an error — empty roster falls back to OVR/age signals.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StarRoster::default()),
        Err(e) => return Err(e.into()),
    };

    let parsed: std::collections::HashMap<String, TeamEntry> = toml::from_str(&text)?;
    let mut by_team = std::collections::HashMap::with_capacity(parsed.len());
    for (abbrev, entry) in parsed {
        if entry.players.is_empty() {
            continue;
        }
        by_team.insert(abbrev.to_ascii_uppercase(), entry.players);
    }
    Ok(StarRoster { by_team })
}

pub fn star_protection(
    player: PlayerId,
    owning_team: TeamId,
    league: &LeagueSnapshot,
    roster: &StarRoster,
    weights: &StarProtectionWeights,
) -> Score {
    let mut score = Score::new(0.0);

    // Player must exist and be on the claimed team. If not, return a neutral
    // 0.0 — caller is asking about a player that doesn't belong to this team,
    // which means there's no franchise relationship to protect.
    let Some(p) = league.player(player) else {
        score.add("player_missing", 0.0);
        return score;
    };
    if p.team != Some(owning_team) {
        score.add("not_on_team", 0.0);
        return score;
    }
    let Some(team) = league.team(owning_team) else {
        score.add("team_missing", 0.0);
        return score;
    };

    // ---- Component 1: franchise_tag.
    // The headline override. If listed, this alone hits the untouchable
    // threshold so the trade engine never asks "what's Luka worth?".
    if roster.is_tagged(&team.abbrev, &p.name) {
        score.add("franchise_tag", weights.franchise_tag_value as f64);
    }

    // ---- Component 2: top_ovr_on_team.
    // Highest OVR on the roster gets a flat bump. Amplified for Contend mode
    // (a contender's #1 is the most untouchable player in the league),
    // dampened for FullRebuild (everyone is fair game during a teardown).
    let team_roster = league.roster(owning_team);
    let team_max_ovr = team_roster.iter().map(|q| q.overall).max().unwrap_or(0);
    let is_top_on_team = team_max_ovr > 0 && p.overall >= team_max_ovr;

    let team_mode_weights = TeamContextWeights::default();
    let context = team_context(owning_team, league, &team_mode_weights);

    if is_top_on_team {
        let bump = match context.mode {
            TeamMode::Contend => weights.top_ovr_bump as f64 * 1.25,
            TeamMode::FullRebuild => weights.top_ovr_bump as f64 * 0.5,
            TeamMode::Tank => weights.top_ovr_bump as f64 * 0.6,
            _ => weights.top_ovr_bump as f64,
        };
        score.add("top_ovr_on_team", bump);
    }

    // ---- Component 3: young_ascending.
    // 24-and-under with potential ≥ 90 — the next franchise cornerstone.
    if p.age <= 24 && p.potential >= 90 {
        score.add("young_ascending", weights.young_ascending_bump as f64);
    }

    // ---- Component 4: recent_signing.
    // Extension/signing inside the last sim year. The current LeagueSnapshot
    // exposes `current_season`; we treat "signed this season or last" as
    // recent. Cheap and good enough — full sim-month tracking is M5+.
    if let Some(contract) = &p.contract {
        let current = league.current_season.0 as i32;
        let signed = contract.signed_in_season.0 as i32;
        if signed >= current - 1 {
            score.add("recent_signing", weights.recent_signing_bump as f64);
        }
    }

    // ---- Component 5: team_mode_modifier.
    // Pure rebuild → push protection down for non-tagged players (clearout
    // mode). Already baked into the top_ovr bump above; here we apply a
    // small floor adjustment so a FullRebuild's top_ovr never accidentally
    // crosses the absolute_threshold without an explicit franchise tag.
    if matches!(context.mode, TeamMode::FullRebuild)
        && score.value < weights.absolute_threshold as f64
    {
        let pull = -0.05_f64;
        score.add("team_mode_full_rebuild", pull);
    }

    // Clamp the headline value to [0, 1] without retroactively rewriting
    // each delta — reasons are still individually informative.
    if score.value < 0.0 {
        score.value = 0.0;
    }
    if score.value > 1.0 {
        score.value = 1.0;
    }

    score.sort_reasons();
    score
}
