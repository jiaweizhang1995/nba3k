//! `nba3k-scrape` — produce a seeded SQLite from public NBA sources.
//!
//! See `crates/nba3k-scrape/README.md` for the full pipeline + Python
//! `nba_api` install requirement.

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
        RawPlayerStats,
    },
    teams::TEAMS,
};

#[derive(Parser)]
#[command(name = "nba3k-scrape", about = "Produce seed SQLite from public NBA data sources")]
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

    // Stage 1: BBRef rosters per team (best-effort; fall back to a
    // synthetic generator if the network/cache yields nothing).
    let bbref = BbrefSource {
        fetcher: &fetcher,
        cache: &cache,
        end_year: season.0,
    };

    let mut rated_by_team: HashMap<u8, Vec<ratings::RatedPlayer>> = HashMap::new();
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

        let rated = ratings::rate_all(&players);
        rated_by_team.insert(team.id, rated);
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
    let contracts = match (HoopsHypeSource { fetcher: &fetcher, cache: &cache }.fetch_all()) {
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
