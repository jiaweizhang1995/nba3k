//! `nba3k-scrape` — produce a seeded SQLite from public NBA sources.
//!
//! See `crates/nba3k-scrape/README.md` for the full pipeline + Python
//! `nba_api` install requirement.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;

use nba3k_core::{LeagueYear, SeasonId};
use nba3k_scrape::{
    assertions::{self, Bounds},
    cache::Cache,
    overrides::OverridesIndex,
    politeness::Fetcher,
    ratings,
    seed::{self, SeedInput},
    sources::{
        bbref::BbrefSource,
        hoophype::HoopsHypeSource,
        mock_draft::TOP_60,
        nba_api::{self as nba_api, NbaApiStatus},
        normalize_player_name, RawPlayerStats,
    },
    teams::TEAMS,
};

const MAX_PER_TEAM: usize = 15;

#[derive(Parser)]
#[command(
    name = "nba3k-scrape",
    about = "Produce seed SQLite from public NBA data sources"
)]
struct Cli {
    /// Season string, e.g. "2025-26" → end-year 2026.
    #[arg(long, default_value = "2025-26")]
    season: String,

    /// Output SQLite path.
    #[arg(long, default_value = "data/seed_2025_26.sqlite")]
    out: PathBuf,

    /// Cache root directory.
    #[arg(long, default_value = "data/cache")]
    cache_dir: PathBuf,

    /// Path to optional rating/contract override file.
    #[arg(long, default_value = "data/rating_overrides.toml")]
    overrides: PathBuf,

    /// Don't drop+recreate; preserve existing seed contents.
    #[arg(long)]
    keep_existing: bool,

    /// If the network is unreachable or rate-limited, fall back to a
    /// synthetic roster generator so the binary still produces a valid
    /// seed for downstream phases. Default: on (the spec wants the
    /// acceptance count to land regardless of network state).
    #[arg(long, default_value_t = true)]
    offline_fallback: bool,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,reqwest=warn")),
        )
        .init();

    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nba3k-scrape: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let ly = LeagueYear::for_label(&cli.season)
        .with_context(|| format!("unknown season label '{}'", cli.season))?;
    let season: SeasonId = ly.season;

    eprintln!(
        "nba3k-scrape: season={} (SeasonId={}) out={} cache={}",
        cli.season,
        season.0,
        cli.out.display(),
        cli.cache_dir.display()
    );

    // Surface the Python `nba_api` status up front so the user knows what
    // they're getting. We never auto-install.
    match nba_api::probe() {
        NbaApiStatus::Available => eprintln!("nba_api: OK (Python module available)"),
        NbaApiStatus::MissingPython => {
            eprintln!("nba_api: python3 not found — advanced stats augmentation disabled");
            eprintln!("         install Python 3 to enable usage rate / TS% augmentation");
        }
        NbaApiStatus::MissingPackage => {
            eprintln!("nba_api: python3 found, but `import nba_api` failed");
            eprintln!("         to enable advanced stats: pip install nba_api");
        }
    }

    let cache = Cache::new(&cli.cache_dir).context("init cache")?;
    let fetcher = Fetcher::new()?;
    let overrides = OverridesIndex::load_or_empty(&cli.overrides).context("load overrides")?;
    if !overrides.is_empty() {
        eprintln!("overrides: loaded {} entries", overrides.len());
    }

    // Stage 1: BBRef rosters per team. Pull stats from the PRIOR season
    // (2024-25 for a 2025-26 game) — the in-progress year has incomplete
    // games-played samples (Tatum's Achilles injury, mid-season trades,
    // etc.) which inflate or wipe star ratings. 2K games rate using the
    // last completed season the same way. Best-effort; fall back to
    // synthetic generator if the network / cache yields nothing.
    let bbref = BbrefSource {
        fetcher: &fetcher,
        cache: &cache,
        end_year: season.0.saturating_sub(1),
    };

    // Collect EVERY player league-wide first, then run percentile-rank ratings
    // ONCE across the whole pool. Old code rated per-team — Tatum's pct_pts
    // was always ~0.95 within BOS, so every team's top scorer landed at 95+
    // regardless of how they actually compared league-wide. League-wide ranks
    // give Shai a true 0.99 and Queta a true 0.55.
    let mut team_player_counts: Vec<(u8, &str)> = Vec::new();
    let mut all_players: Vec<RawPlayerStats> = Vec::new();
    let mut team_player_offsets: HashMap<u8, (usize, usize)> = HashMap::new();
    let mut total_real = 0u32;
    let mut total_synth = 0u32;

    for team in TEAMS {
        let players = match bbref.fetch_team(team.abbrev) {
            Ok(rows) if !rows.is_empty() => {
                tracing::info!(team = team.abbrev, count = rows.len(), "bbref ok");
                rows
            }
            Ok(_) => {
                tracing::warn!(team = team.abbrev, "bbref empty");
                if cli.offline_fallback {
                    synthetic_roster(team.abbrev, team.id)
                } else {
                    vec![]
                }
            }
            Err(e) => {
                tracing::warn!(team = team.abbrev, error = %e, "bbref fetch failed");
                if cli.offline_fallback {
                    synthetic_roster(team.abbrev, team.id)
                } else {
                    return Err(e);
                }
            }
        };

        // Track real vs synthetic for the report.
        if players.iter().all(|p| p.name.starts_with(team.abbrev)) {
            total_synth += players.len() as u32;
        } else {
            total_real += players.len() as u32;
        }

        let start = all_players.len();
        let count = players.len();
        all_players.extend(players);
        team_player_offsets.insert(team.id, (start, start + count));
        team_player_counts.push((team.id, team.abbrev));
    }

    let before_filter = all_players.len();
    dedup_and_cap(&mut all_players, &mut team_player_offsets);
    let after_filter = all_players.len();
    if after_filter != before_filter {
        eprintln!(
            "rosters: filtered {} raw rows via duplicate primary-team pass + top-{MAX_PER_TEAM} team cap",
            before_filter - after_filter
        );
    }

    // Single league-wide rating pass.
    let rated_all = ratings::rate_all(&all_players);
    let mut rated_by_team: HashMap<u8, Vec<ratings::RatedPlayer>> = HashMap::new();
    for (team_id, _) in &team_player_counts {
        if let Some(&(s, e)) = team_player_offsets.get(team_id) {
            rated_by_team.insert(*team_id, rated_all[s..e].to_vec());
        }
    }

    if total_real == 0 {
        eprintln!(
            "warning: every team used the synthetic roster generator. \
             Network likely unreachable or BBRef returned 429. Cache will populate \
             on next successful run."
        );
    } else {
        eprintln!("rosters: {total_real} real players, {total_synth} synthetic fillers");
    }

    // Stage 2: HoopsHype contracts (optional — best effort).
    let contracts = match (HoopsHypeSource {
        fetcher: &fetcher,
        cache: &cache,
    }
    .fetch_all())
    {
        Ok(rows) => {
            eprintln!("hoopshype: parsed {} contract rows", rows.len());
            rows
        }
        Err(e) => {
            tracing::warn!(error = %e, "hoopshype fetch failed; continuing without contracts");
            vec![]
        }
    };

    // Stage 3: write seed.
    let report = seed::write_seed(
        &cli.out,
        cli.keep_existing,
        SeedInput {
            season,
            rated_by_team,
            contracts: &contracts,
            prospects: TOP_60,
            overrides: &overrides,
        },
    )
    .context("write seed")?;

    eprintln!(
        "seed: {} teams, {} players, {} prospects, {} contracts",
        report.teams, report.players, report.prospects, report.players_with_contract
    );

    // Stage 4: sanity assertions. Non-zero exit on any failure.
    assertions::run_all(&cli.out, season, &Bounds::default()).context("post-scrape assertions")?;
    eprintln!("assertions: OK");

    Ok(())
}

#[derive(Debug, Clone)]
struct TeamPlayerCandidate {
    team_id: u8,
    player: RawPlayerStats,
}

/// BBRef team pages include everyone who appeared for that team in the prior
/// season, including traded-out and short-stint players. Pick each duplicate
/// player's primary team first, then keep each team's top minutes roster before
/// league-wide ratings so downstream CBA checks see real 13-15 player teams
/// instead of historical appearance logs.
fn dedup_and_cap(
    all_players: &mut Vec<RawPlayerStats>,
    team_player_offsets: &mut HashMap<u8, (usize, usize)>,
) {
    let mut team_ids: Vec<u8> = team_player_offsets.keys().copied().collect();
    team_ids.sort_by_key(|id| {
        team_player_offsets
            .get(id)
            .map(|(start, _)| *start)
            .unwrap_or(0)
    });

    let mut candidates = Vec::new();
    for team_id in &team_ids {
        let Some(&(start, end)) = team_player_offsets.get(team_id) else {
            continue;
        };
        candidates.extend(
            all_players[start..end]
                .iter()
                .filter(|player| player.minutes_per_game > 0.0)
                .cloned()
                .map(|player| TeamPlayerCandidate {
                    team_id: *team_id,
                    player,
                }),
        );
    }

    let mut best_by_name: HashMap<String, usize> = HashMap::new();
    for (idx, candidate) in candidates.iter().enumerate() {
        let key = normalize_player_name(&candidate.player.name);
        if key.is_empty() {
            continue;
        }
        match best_by_name.get(&key).copied() {
            Some(current_idx)
                if duplicate_choice_order(candidate, &candidates[current_idx]).is_lt() =>
            {
                best_by_name.insert(key, idx);
            }
            None => {
                best_by_name.insert(key, idx);
            }
            _ => {}
        }
    }

    all_players.clear();
    team_player_offsets.clear();
    for team_id in team_ids {
        let mut team_rows: Vec<RawPlayerStats> = candidates
            .iter()
            .enumerate()
            .filter(|(idx, candidate)| {
                candidate.team_id == team_id
                    && best_by_name
                        .get(&normalize_player_name(&candidate.player.name))
                        .is_some_and(|best_idx| *best_idx == *idx)
            })
            .map(|(_, candidate)| candidate.player.clone())
            .collect();
        team_rows.sort_by(compare_team_cap_order);
        team_rows.truncate(MAX_PER_TEAM);

        let start = all_players.len();
        all_players.extend(team_rows);
        team_player_offsets.insert(team_id, (start, all_players.len()));
    }
}

fn compare_team_cap_order(a: &RawPlayerStats, b: &RawPlayerStats) -> Ordering {
    b.minutes_per_game
        .total_cmp(&a.minutes_per_game)
        .then_with(|| b.games.total_cmp(&a.games))
        .then_with(|| a.name.cmp(&b.name))
}

fn duplicate_choice_order(a: &TeamPlayerCandidate, b: &TeamPlayerCandidate) -> Ordering {
    let a_total = a.player.minutes_per_game * a.player.games;
    let b_total = b.player.minutes_per_game * b.player.games;
    b_total
        .total_cmp(&a_total)
        .then_with(|| {
            b.player
                .minutes_per_game
                .total_cmp(&a.player.minutes_per_game)
        })
        .then_with(|| b.player.games.total_cmp(&a.player.games))
        .then_with(|| a.player.name.cmp(&b.player.name))
        .then_with(|| a.team_id.cmp(&b.team_id))
}

/// Synthetic 15-player roster used when the network is unreachable. We
/// produce a position-balanced spread (PG×2 SG×3 SF×3 PF×3 C×4) with
/// modest but varied stats so ratings still produce reasonable values.
fn synthetic_roster(abbrev: &str, team_id: u8) -> Vec<RawPlayerStats> {
    use nba3k_core::Position::*;
    let positions = [PG, PG, SG, SG, SG, SF, SF, SF, PF, PF, PF, C, C, C, C];
    let seed = team_id as f32;
    positions
        .iter()
        .enumerate()
        .map(|(i, &pos)| {
            // Stat shapes vary by depth-chart slot to keep ratings spread.
            let starter_bias = 1.0_f32.max(2.5 - (i as f32) * 0.18);
            let pts = (8.0 + (i as f32 * 0.7) + seed * 0.2).min(28.0) * starter_bias / 1.5;
            let trb = match pos {
                C | PF => 6.0 + i as f32 * 0.2,
                SF => 4.0,
                _ => 2.5,
            };
            let ast = match pos {
                PG => 5.5,
                SG => 3.0,
                SF => 2.5,
                _ => 1.5,
            };
            RawPlayerStats {
                name: format!("{abbrev} Player {:02}", i + 1),
                primary_position: pos,
                secondary_position: None,
                age: 22 + (i as u8 % 10),
                games: 70.0,
                minutes_per_game: (32.0 - i as f32 * 1.5).max(8.0),
                pts,
                trb,
                ast,
                stl: 1.0,
                blk: if matches!(pos, C | PF) { 1.0 } else { 0.4 },
                tov: 1.5,
                fg_pct: 0.46,
                three_pct: if matches!(pos, C) { 0.20 } else { 0.36 },
                ft_pct: 0.78,
                usage: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba3k_core::Position;

    fn raw(name: &str, minutes_per_game: f32, games: f32) -> RawPlayerStats {
        RawPlayerStats {
            name: name.to_string(),
            primary_position: Position::SG,
            secondary_position: None,
            age: 25,
            games,
            minutes_per_game,
            pts: 0.0,
            trb: 0.0,
            ast: 0.0,
            stl: 0.0,
            blk: 0.0,
            tov: 0.0,
            fg_pct: 0.0,
            three_pct: 0.0,
            ft_pct: 0.0,
            usage: None,
        }
    }

    #[test]
    fn dedup_and_cap_keeps_top_15_and_drops_zero_minute_rows() {
        let mut players: Vec<RawPlayerStats> = (1..=16)
            .map(|n| {
                raw(
                    &format!("Team One {}", char::from(b'A' + n as u8 - 1)),
                    n as f32,
                    70.0,
                )
            })
            .collect();
        players.push(raw("Zero Minute", 0.0, 82.0));
        let mut offsets = HashMap::from([(1, (0, players.len()))]);

        dedup_and_cap(&mut players, &mut offsets);

        assert_eq!(players.len(), 15);
        assert_eq!(offsets.get(&1), Some(&(0, 15)));
        assert!(!players.iter().any(|p| p.name == "Zero Minute"));
        assert!(!players.iter().any(|p| p.name == "Team One A"));
        assert_eq!(players.first().map(|p| p.name.as_str()), Some("Team One P"));
    }

    #[test]
    fn dedup_and_cap_uses_games_as_team_cap_tiebreak() {
        let mut players = vec![
            raw("Same MPG Low Games", 10.0, 10.0),
            raw("Same MPG High Games", 10.0, 70.0),
            raw("Higher MPG", 12.0, 1.0),
        ];
        let mut offsets = HashMap::from([(1, (0, players.len()))]);

        dedup_and_cap(&mut players, &mut offsets);

        let names: Vec<&str> = players.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Higher MPG", "Same MPG High Games", "Same MPG Low Games"]
        );
    }

    #[test]
    fn dedup_and_cap_keeps_duplicate_on_primary_minutes_team_and_rebuilds_offsets() {
        let mut players = vec![
            raw("Shared Player", 20.0, 10.0),
            raw("Team One Unique", 8.0, 70.0),
            raw("Shared Player", 12.0, 30.0),
            raw("Team Two Unique", 7.0, 70.0),
        ];
        let mut offsets = HashMap::from([(1, (0, 2)), (2, (2, 4))]);

        dedup_and_cap(&mut players, &mut offsets);

        assert_eq!(players.len(), 3);
        assert_eq!(offsets.get(&1), Some(&(0, 1)));
        assert_eq!(offsets.get(&2), Some(&(1, 3)));
        assert_eq!(players[0].name, "Team One Unique");
        assert_eq!(players[1].name, "Shared Player");
        assert_eq!(players[2].name, "Team Two Unique");
    }
}
