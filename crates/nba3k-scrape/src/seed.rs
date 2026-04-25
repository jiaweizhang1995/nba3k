//! Compose teams + rated players + contracts + draft prospects, then
//! write them to a fresh SQLite via `nba3k-store::Store`.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use nba3k_core::{
    BirdRights, Cents, Contract, ContractYear, DraftProspect, Player, PlayerId, Position,
    SeasonId, Team, TeamId,
};
use nba3k_store::Store;

use crate::ids::player_id_from;
use crate::overrides::OverridesIndex;
use crate::ratings::RatedPlayer;
use crate::sources::hoophype::ContractRow;
use crate::sources::mock_draft::MockProspect;
use crate::teams::{build_team, TEAMS};

pub struct SeedInput<'a> {
    pub season: SeasonId,
    pub rated_by_team: HashMap<u8, Vec<RatedPlayer>>,
    pub contracts: &'a [ContractRow],
    pub prospects: &'a [MockProspect],
    pub overrides: &'a OverridesIndex,
}

#[derive(Debug, Default)]
pub struct SeedReport {
    pub teams: u32,
    pub players: u32,
    pub prospects: u32,
    pub players_with_contract: u32,
}

pub fn write_seed(out: &Path, keep_existing: bool, input: SeedInput<'_>) -> Result<SeedReport> {
    if !keep_existing && out.exists() {
        std::fs::remove_file(out).with_context(|| format!("remove existing {out:?}"))?;
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    let mut store = Store::open(out).context("open store")?;
    store.init_metadata(input.season).context("init metadata")?;

    // Teams
    let mut teams: Vec<Team> = TEAMS.iter().copied().map(build_team).collect();
    for team in &teams {
        store.upsert_team(team).context("upsert team")?;
    }
    let team_count = teams.len() as u32;

    // Players
    let mut all_players: Vec<Player> = Vec::new();
    let mut with_contract = 0u32;
    let contracts_by_name = build_contract_index(input.contracts);

    for (team_id_raw, rated) in &input.rated_by_team {
        let team_id = TeamId(*team_id_raw);
        for r in rated {
            let pid = player_id_from(&r.stats.name, r.stats.age as u32, *team_id_raw as u32);
            let mut player = Player {
                id: pid,
                name: r.stats.name.clone(),
                primary_position: r.stats.primary_position,
                secondary_position: r.stats.secondary_position,
                age: r.stats.age,
                overall: r.overall,
                potential: r.potential,
                ratings: r.ratings,
                contract: contract_for(&contracts_by_name, &r.stats.name, input.season),
                team: Some(team_id),
                injury: None,
                no_trade_clause: false,
                trade_kicker_pct: None,
                role: nba3k_core::PlayerRole::default(),
                morale: 0.5,
            };
            apply_override(&mut player, input.overrides);
            if player.contract.is_some() {
                with_contract += 1;
            }
            all_players.push(player);
        }
    }

    // Roster lists for teams (mutate already-stored teams).
    let mut roster_by_team: HashMap<u8, Vec<PlayerId>> = HashMap::new();
    for p in &all_players {
        if let Some(t) = p.team {
            roster_by_team.entry(t.0).or_default().push(p.id);
        }
    }
    for team in &mut teams {
        if let Some(roster) = roster_by_team.remove(&team.id.0) {
            team.roster = roster;
            store.upsert_team(team).context("re-upsert team with roster")?;
        }
    }

    let players_count = all_players.len() as u32;
    store.bulk_upsert_players(&all_players).context("bulk upsert players")?;

    // Prospects
    let mut prospects_count = 0u32;
    for mp in input.prospects {
        let pid = PlayerId(50_000_000 + mp.rank as u32);
        let prospect = DraftProspect {
            id: pid,
            name: mp.name.to_string(),
            mock_rank: mp.rank,
            age: mp.age,
            position: mp.position,
            ratings: ratings_from_potential(mp.potential, mp.position),
            potential: mp.potential,
            draft_class: SeasonId(input.season.0 + 1),
        };
        store.upsert_draft_prospect(&prospect).context("upsert prospect")?;
        prospects_count += 1;
    }

    Ok(SeedReport {
        teams: team_count,
        players: players_count,
        prospects: prospects_count,
        players_with_contract: with_contract,
    })
}

fn build_contract_index(rows: &[ContractRow]) -> HashMap<String, &ContractRow> {
    let mut idx = HashMap::with_capacity(rows.len());
    for row in rows {
        idx.insert(normalize(&row.player_name), row);
    }
    idx
}

fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphabetic())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn contract_for(
    idx: &HashMap<String, &ContractRow>,
    name: &str,
    season: SeasonId,
) -> Option<Contract> {
    let row = idx.get(&normalize(name))?;
    let years: Vec<ContractYear> = row
        .salaries
        .iter()
        .enumerate()
        .filter(|(_, &cents)| cents > 0)
        .map(|(i, &cents)| ContractYear {
            season: SeasonId(season.0 + i as u16),
            salary: Cents(cents),
            guaranteed: true,
            team_option: false,
            player_option: false,
        })
        .collect();
    if years.is_empty() {
        return None;
    }
    Some(Contract {
        years,
        signed_in_season: season,
        bird_rights: BirdRights::Full,
    })
}

fn apply_override(player: &mut Player, overrides: &OverridesIndex) {
    let Some(o) = overrides.get(&player.name) else {
        return;
    };
    if let Some(v) = o.overall {
        player.overall = v;
    }
    if let Some(v) = o.potential {
        player.potential = v;
    }
    if let Some(v) = o.no_trade_clause {
        player.no_trade_clause = v;
    }
    if let Some(v) = o.trade_kicker_pct {
        player.trade_kicker_pct = Some(v);
    }
}

fn ratings_from_potential(potential: u8, pos: Position) -> nba3k_core::Ratings {
    // Prospects start ~12 points below their potential. Initialize all 21
    // attributes at base, then position-tweak.
    let base = potential.saturating_sub(12);
    let bump = base.saturating_add(4).min(99);
    let mut r = nba3k_core::Ratings {
        close_shot: base,
        driving_layup: base,
        driving_dunk: base,
        standing_dunk: base,
        post_control: base,
        mid_range: base,
        three_point: base,
        free_throw: base,
        passing_accuracy: base,
        ball_handle: base,
        speed_with_ball: base,
        interior_defense: base,
        perimeter_defense: base,
        steal: base,
        block: base,
        off_reb: base,
        def_reb: base,
        speed: bump,
        agility: bump,
        strength: base,
        vertical: bump,
    };
    match pos {
        Position::PG => {
            r.ball_handle = r.ball_handle.saturating_add(8).min(99);
            r.passing_accuracy = r.passing_accuracy.saturating_add(6).min(99);
        }
        Position::SG => r.three_point = r.three_point.saturating_add(6).min(99),
        Position::SF => r.speed = r.speed.saturating_add(2).min(99),
        Position::PF => {
            r.off_reb = r.off_reb.saturating_add(6).min(99);
            r.def_reb = r.def_reb.saturating_add(6).min(99);
            r.interior_defense = r.interior_defense.saturating_add(4).min(99);
        }
        Position::C => {
            r.off_reb = r.off_reb.saturating_add(10).min(99);
            r.def_reb = r.def_reb.saturating_add(10).min(99);
            r.interior_defense = r.interior_defense.saturating_add(10).min(99);
            r.block = r.block.saturating_add(6).min(99);
            r.three_point = r.three_point.saturating_sub(8);
        }
    }
    r
}
