use crate::StoreResult;
use nba3k_core::*;
use nba3k_models::progression::PlayerDevelopment;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};

pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const SCHEMA_VERSION: u32 = 1;

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

impl Store {
    /// Open existing or create a fresh DB. Runs migrations idempotently.
    pub fn open<P: AsRef<Path>>(path: P) -> StoreResult<Self> {
        let path = path.as_ref().to_path_buf();
        let mut conn = Connection::open(&path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&mut conn)?;
        Ok(Self { conn, path })
    }

    fn migrate(conn: &mut Connection) -> StoreResult<()> {
        crate::embedded::migrations::runner().run(conn)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn conn_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    // ------------------------------------------------------------------
    // meta
    // ------------------------------------------------------------------

    pub fn set_meta(&self, key: &str, value: &str) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> StoreResult<Option<String>> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |r| r.get(0))
            .optional()?;
        Ok(v)
    }

    pub fn init_metadata(&self, season: SeasonId) -> StoreResult<()> {
        self.set_meta("app_version", APP_VERSION)?;
        self.set_meta("schema_version", &SCHEMA_VERSION.to_string())?;
        self.set_meta("season", &season.0.to_string())?;
        self.set_meta("created_at", &chrono::Utc::now().to_rfc3339())?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // season_state
    // ------------------------------------------------------------------

    pub fn save_season_state(&self, state: &SeasonState) -> StoreResult<()> {
        let json = serde_json::to_string(state)?;
        self.conn.execute(
            "INSERT INTO season_state(singleton, state_json) VALUES (0, ?1)
             ON CONFLICT(singleton) DO UPDATE SET state_json = excluded.state_json",
            params![json],
        )?;
        Ok(())
    }

    pub fn load_season_state(&self) -> StoreResult<Option<SeasonState>> {
        let json: Option<String> = self
            .conn
            .query_row("SELECT state_json FROM season_state WHERE singleton = 0", [], |r| {
                r.get(0)
            })
            .optional()?;
        Ok(json.map(|j| serde_json::from_str(&j)).transpose()?)
    }

    // ------------------------------------------------------------------
    // teams
    // ------------------------------------------------------------------

    pub fn upsert_team(&self, team: &Team) -> StoreResult<()> {
        let conf = format!("{:?}", team.conference);
        let div = format!("{:?}", team.division);
        let gm = serde_json::to_string(&team.gm)?;
        self.conn.execute(
            "INSERT INTO teams(id, abbrev, city, name, conference, division, gm_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               abbrev = excluded.abbrev,
               city = excluded.city,
               name = excluded.name,
               conference = excluded.conference,
               division = excluded.division,
               gm_json = excluded.gm_json",
            params![team.id.0 as i64, team.abbrev, team.city, team.name, conf, div, gm],
        )?;
        Ok(())
    }

    pub fn count_teams(&self) -> StoreResult<u32> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM teams", [], |r| r.get(0))?;
        Ok(n as u32)
    }

    pub fn count_players(&self) -> StoreResult<u32> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM players", [], |r| r.get(0))?;
        Ok(n as u32)
    }

    pub fn find_team_by_abbrev(&self, abbrev: &str) -> StoreResult<Option<TeamId>> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM teams WHERE upper(abbrev) = upper(?1)",
                params![abbrev],
                |r| r.get(0),
            )
            .optional()?;
        Ok(id.map(|n| TeamId(n as u8)))
    }

    pub fn player_name(&self, id: PlayerId) -> StoreResult<Option<String>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT name FROM players WHERE id = ?1",
                params![id.0 as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v)
    }

    pub fn team_abbrev(&self, id: TeamId) -> StoreResult<Option<String>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT abbrev FROM teams WHERE id = ?1",
                params![id.0 as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v)
    }

    // ------------------------------------------------------------------
    // players
    // ------------------------------------------------------------------

    pub fn upsert_player(&self, p: &Player) -> StoreResult<()> {
        let primary = p.primary_position.to_string();
        let secondary = p.secondary_position.map(|x| x.to_string());
        let ratings = serde_json::to_string(&p.ratings)?;
        let contract = p.contract.as_ref().map(serde_json::to_string).transpose()?;
        let injury = p.injury.as_ref().map(serde_json::to_string).transpose()?;
        let team_id = p.team.map(|t| t.0 as i64);
        self.conn.execute(
            "INSERT INTO players(
                id, name, primary_position, secondary_position, age,
                overall, potential, ratings_json, contract_json, team_id,
                injury_json, no_trade_clause, trade_kicker_pct
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                primary_position = excluded.primary_position,
                secondary_position = excluded.secondary_position,
                age = excluded.age,
                overall = excluded.overall,
                potential = excluded.potential,
                ratings_json = excluded.ratings_json,
                contract_json = excluded.contract_json,
                team_id = excluded.team_id,
                injury_json = excluded.injury_json,
                no_trade_clause = excluded.no_trade_clause,
                trade_kicker_pct = excluded.trade_kicker_pct",
            params![
                p.id.0 as i64,
                p.name,
                primary,
                secondary,
                p.age as i64,
                p.overall as i64,
                p.potential as i64,
                ratings,
                contract,
                team_id,
                injury,
                if p.no_trade_clause { 1_i64 } else { 0 },
                p.trade_kicker_pct.map(|n| n as i64),
            ],
        )?;
        Ok(())
    }

    /// Bulk player upsert in a single transaction. Faster + atomic.
    pub fn bulk_upsert_players(&mut self, players: &[Player]) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        for p in players {
            let primary = p.primary_position.to_string();
            let secondary = p.secondary_position.map(|x| x.to_string());
            let ratings = serde_json::to_string(&p.ratings)?;
            let contract = p.contract.as_ref().map(serde_json::to_string).transpose()?;
            let injury = p.injury.as_ref().map(serde_json::to_string).transpose()?;
            let team_id = p.team.map(|t| t.0 as i64);
            tx.execute(
                "INSERT INTO players(
                    id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)
                 ON CONFLICT(id) DO UPDATE SET
                    name = excluded.name,
                    primary_position = excluded.primary_position,
                    secondary_position = excluded.secondary_position,
                    age = excluded.age,
                    overall = excluded.overall,
                    potential = excluded.potential,
                    ratings_json = excluded.ratings_json,
                    contract_json = excluded.contract_json,
                    team_id = excluded.team_id,
                    injury_json = excluded.injury_json,
                    no_trade_clause = excluded.no_trade_clause,
                    trade_kicker_pct = excluded.trade_kicker_pct",
                params![
                    p.id.0 as i64,
                    p.name,
                    primary,
                    secondary,
                    p.age as i64,
                    p.overall as i64,
                    p.potential as i64,
                    ratings,
                    contract,
                    team_id,
                    injury,
                    if p.no_trade_clause { 1_i64 } else { 0 },
                    p.trade_kicker_pct.map(|n| n as i64),
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // player development (M5 progression engine)
    //
    // Stored as JSON in `players.dev_json` (V003). NULL means "not yet
    // initialized" — `read_player_dev` bootstraps with defaults rather
    // than failing, so existing seed DBs work without a backfill step.
    // ------------------------------------------------------------------

    /// Read PlayerDevelopment for a player. Returns `Ok(None)` if the
    /// player doesn't exist; bootstraps a default development record if
    /// the player exists but `dev_json` is NULL.
    pub fn read_player_dev(
        &self,
        id: PlayerId,
        season: SeasonId,
    ) -> StoreResult<Option<PlayerDevelopment>> {
        let row: Option<(Option<String>, i64, i64, i64)> = self
            .conn
            .query_row(
                "SELECT dev_json, age, overall, potential
                 FROM players WHERE id = ?1",
                params![id.0 as i64],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((json_opt, _age, _ovr, potential)) = row else {
            return Ok(None);
        };
        if let Some(json) = json_opt {
            let dev: PlayerDevelopment = serde_json::from_str(&json)?;
            return Ok(Some(dev));
        }
        // Bootstrap default for seed players. Mirrors
        // `PlayerDevelopment::defaults_for` but built from the row alone
        // so we don't have to round-trip the full Player.
        Ok(Some(PlayerDevelopment {
            player_id: id,
            peak_start_age: 25,
            peak_end_age: 30,
            dynamic_potential: potential as u8,
            work_ethic: 70,
            last_progressed_season: season,
        }))
    }

    pub fn write_player_dev(&self, dev: &PlayerDevelopment) -> StoreResult<()> {
        let json = serde_json::to_string(dev)?;
        self.conn.execute(
            "UPDATE players SET dev_json = ?1 WHERE id = ?2",
            params![json, dev.player_id.0 as i64],
        )?;
        Ok(())
    }

    /// Bulk write PlayerDevelopment for many players in one transaction.
    /// Used by the season-end progression pass.
    pub fn bulk_upsert_player_dev(&mut self, devs: &[PlayerDevelopment]) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        for dev in devs {
            let json = serde_json::to_string(dev)?;
            tx.execute(
                "UPDATE players SET dev_json = ?1 WHERE id = ?2",
                params![json, dev.player_id.0 as i64],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // draft prospects
    //
    // Prospects are stored in `players` with team_id = NULL. The seed-time
    // `is_prospect` distinction is "player has no team and is in the current
    // draft class" — adequate for v1. M5 will overlay a draft_class table.
    // ------------------------------------------------------------------

    pub fn upsert_draft_prospect(&self, prospect: &DraftProspect) -> StoreResult<()> {
        let ratings = serde_json::to_string(&prospect.ratings)?;
        self.conn.execute(
            "INSERT INTO players(
                id, name, primary_position, secondary_position, age,
                overall, potential, ratings_json, contract_json, team_id,
                injury_json, no_trade_clause, trade_kicker_pct
             ) VALUES (?1,?2,?3,NULL,?4,?5,?6,?7,NULL,NULL,NULL,0,NULL)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                primary_position = excluded.primary_position,
                age = excluded.age,
                overall = excluded.overall,
                potential = excluded.potential,
                ratings_json = excluded.ratings_json,
                team_id = NULL",
            params![
                prospect.id.0 as i64,
                prospect.name,
                prospect.position.to_string(),
                prospect.age as i64,
                prospect.ratings.overall_estimate() as i64,
                prospect.potential as i64,
                ratings,
            ],
        )?;
        Ok(())
    }

    pub fn count_prospects(&self) -> StoreResult<u32> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM players WHERE team_id IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    // ------------------------------------------------------------------
    // teams (read)
    // ------------------------------------------------------------------

    pub fn list_teams(&self) -> StoreResult<Vec<Team>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, abbrev, city, name, conference, division, gm_json
             FROM teams ORDER BY id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let abbrev: String = r.get(1)?;
                let city: String = r.get(2)?;
                let name: String = r.get(3)?;
                let conf_str: String = r.get(4)?;
                let div_str: String = r.get(5)?;
                let gm_json: String = r.get(6)?;
                Ok((id, abbrev, city, name, conf_str, div_str, gm_json))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut out = Vec::with_capacity(rows.len());
        for (id, abbrev, city, name, conf_str, div_str, gm_json) in rows {
            let conference = parse_conference(&conf_str);
            let division = parse_division(&div_str);
            let gm: GMPersonality = serde_json::from_str(&gm_json)?;
            let coach = Coach::default_for(&abbrev);
            out.push(Team {
                id: TeamId(id as u8),
                abbrev,
                city,
                name,
                conference,
                division,
                gm,
                coach,
                roster: vec![],
                draft_picks: vec![],
            });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // players (read)
    // ------------------------------------------------------------------

    pub fn roster_for_team(&self, team: TeamId) -> StoreResult<Vec<Player>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct
             FROM players WHERE team_id = ?1 ORDER BY overall DESC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![team.0 as i64], read_player_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(deserialize_player).collect()
    }

    pub fn find_player_by_name(&self, name: &str) -> StoreResult<Option<Player>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, name, primary_position, secondary_position, age,
                        overall, potential, ratings_json, contract_json, team_id,
                        injury_json, no_trade_clause, trade_kicker_pct
                 FROM players
                 WHERE lower(name) = lower(?1)
                    OR lower(name) LIKE lower(?2)
                 ORDER BY overall DESC LIMIT 1",
                params![name, format!("%{}%", name)],
                read_player_row,
            )
            .optional()?;
        match row {
            Some(r) => Ok(Some(deserialize_player(r)?)),
            None => Ok(None),
        }
    }

    // ------------------------------------------------------------------
    // schedule
    // ------------------------------------------------------------------

    pub fn bulk_insert_schedule(
        &mut self,
        rows: &[(u64, SeasonId, chrono::NaiveDate, TeamId, TeamId)],
    ) -> StoreResult<()> {
        let tx = self.conn.transaction()?;
        for (game_id, season, date, home, away) in rows {
            tx.execute(
                "INSERT INTO schedule(game_id, season, date, home, away, played, is_playoffs)
                 VALUES (?1, ?2, ?3, ?4, ?5, 0, 0)
                 ON CONFLICT(game_id) DO UPDATE SET
                    season=excluded.season, date=excluded.date,
                    home=excluded.home, away=excluded.away",
                params![
                    *game_id as i64,
                    season.0 as i64,
                    date.to_string(),
                    home.0 as i64,
                    away.0 as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn count_schedule(&self) -> StoreResult<u32> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM schedule", [], |r| r.get(0))?;
        Ok(n as u32)
    }

    pub fn pending_games_through(
        &self,
        through_date: chrono::NaiveDate,
    ) -> StoreResult<Vec<ScheduledRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT game_id, season, date, home, away
             FROM schedule
             WHERE played = 0 AND date <= ?1
             ORDER BY date ASC, game_id ASC",
        )?;
        let rows = stmt
            .query_map(params![through_date.to_string()], |r| {
                let id: i64 = r.get(0)?;
                let season: i64 = r.get(1)?;
                let date_str: String = r.get(2)?;
                let home: i64 = r.get(3)?;
                let away: i64 = r.get(4)?;
                Ok(ScheduledRow {
                    game_id: id as u64,
                    season: SeasonId(season as u16),
                    date: chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
                        .unwrap_or(through_date),
                    home: TeamId(home as u8),
                    away: TeamId(away as u8),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn first_unplayed_date(&self) -> StoreResult<Option<chrono::NaiveDate>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT MIN(date) FROM schedule WHERE played = 0",
                [],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        Ok(v.and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()))
    }

    pub fn last_scheduled_date(&self) -> StoreResult<Option<chrono::NaiveDate>> {
        let v: Option<String> = self
            .conn
            .query_row("SELECT MAX(date) FROM schedule", [], |r| r.get(0))
            .optional()?
            .flatten();
        Ok(v.and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok()))
    }

    pub fn count_unplayed(&self) -> StoreResult<u32> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM schedule WHERE played = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    pub fn record_game(&self, g: &GameResult) -> StoreResult<()> {
        let box_score = serde_json::to_string(&g.box_score)?;
        self.conn.execute(
            "INSERT INTO games(id, season, date, home, away, home_score, away_score,
                               overtime_periods, is_playoffs, box_score_json)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
             ON CONFLICT(id) DO UPDATE SET
                home_score = excluded.home_score,
                away_score = excluded.away_score,
                overtime_periods = excluded.overtime_periods,
                box_score_json = excluded.box_score_json",
            params![
                g.id.0 as i64,
                g.season.0 as i64,
                g.date.to_string(),
                g.home.0 as i64,
                g.away.0 as i64,
                g.home_score as i64,
                g.away_score as i64,
                g.overtime_periods as i64,
                if g.is_playoffs { 1_i64 } else { 0 },
                box_score
            ],
        )?;
        self.conn.execute(
            "UPDATE schedule SET played = 1 WHERE game_id = ?1",
            params![g.id.0 as i64],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // standings
    // ------------------------------------------------------------------

    pub fn upsert_standing(
        &self,
        team: TeamId,
        season: SeasonId,
        wins: u16,
        losses: u16,
        conf_rank: Option<u8>,
    ) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO standings(team_id, season, wins, losses, conf_rank)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(team_id, season) DO UPDATE SET
                wins = excluded.wins,
                losses = excluded.losses,
                conf_rank = excluded.conf_rank",
            params![
                team.0 as i64,
                season.0 as i64,
                wins as i64,
                losses as i64,
                conf_rank.map(|n| n as i64)
            ],
        )?;
        Ok(())
    }

    pub fn read_games(&self, season: SeasonId) -> StoreResult<Vec<GameResult>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, season, date, home, away, home_score, away_score,
                    overtime_periods, is_playoffs, box_score_json
             FROM games WHERE season = ?1
             ORDER BY date ASC, id ASC",
        )?;
        let raw = stmt
            .query_map(params![season.0 as i64], |r| {
                let id: i64 = r.get(0)?;
                let season: i64 = r.get(1)?;
                let date_s: String = r.get(2)?;
                let home: i64 = r.get(3)?;
                let away: i64 = r.get(4)?;
                let home_score: i64 = r.get(5)?;
                let away_score: i64 = r.get(6)?;
                let ot: i64 = r.get(7)?;
                let pl: i64 = r.get(8)?;
                let box_json: String = r.get(9)?;
                Ok((id, season, date_s, home, away, home_score, away_score, ot, pl, box_json))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut out = Vec::with_capacity(raw.len());
        for (id, season_n, date_s, home, away, hs, as_, ot, pl, box_json) in raw {
            let box_score: BoxScore = serde_json::from_str(&box_json)?;
            out.push(GameResult {
                id: GameId(id as u64),
                season: SeasonId(season_n as u16),
                date: chrono::NaiveDate::parse_from_str(&date_s, "%Y-%m-%d").unwrap_or_default(),
                home: TeamId(home as u8),
                away: TeamId(away as u8),
                home_score: hs as u16,
                away_score: as_ as u16,
                overtime_periods: ot as u8,
                is_playoffs: pl != 0,
                box_score,
            });
        }
        Ok(out)
    }

    pub fn scheduled_games_per_team(&self) -> StoreResult<std::collections::HashMap<TeamId, u32>> {
        let mut stmt = self.conn.prepare(
            "SELECT team, COUNT(*) FROM (
                 SELECT home AS team FROM schedule
                 UNION ALL
                 SELECT away FROM schedule
             ) GROUP BY team",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let n: i64 = r.get(1)?;
                Ok((TeamId(id as u8), n as u32))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows.into_iter().collect())
    }

    // ------------------------------------------------------------------
    // bulk readers used by trade snapshot construction
    // ------------------------------------------------------------------

    pub fn all_active_players(&self) -> StoreResult<Vec<Player>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct
             FROM players WHERE team_id IS NOT NULL",
        )?;
        let rows = stmt
            .query_map([], read_player_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(deserialize_player).collect()
    }

    pub fn all_picks(&self) -> StoreResult<Vec<DraftPick>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, original_team, current_owner, season, round, protections_json
             FROM draft_picks ORDER BY season, round, current_owner",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let id: i64 = r.get(0)?;
                let original: i64 = r.get(1)?;
                let owner: i64 = r.get(2)?;
                let season: i64 = r.get(3)?;
                let round: i64 = r.get(4)?;
                let protections_json: String = r.get(5)?;
                Ok((id, original, owner, season, round, protections_json))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, original, owner, season, round, protections_json) in rows {
            let protections: Protection = serde_json::from_str(&protections_json)?;
            out.push(DraftPick {
                id: DraftPickId(id as u32),
                original_team: TeamId(original as u8),
                current_owner: TeamId(owner as u8),
                season: SeasonId(season as u16),
                round: round as u8,
                protections,
            });
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // trade history
    // ------------------------------------------------------------------

    /// Persist a negotiation state. Returns the assigned trade id.
    /// `accepted` is 1 only when the chain ended in `NegotiationState::Accepted`.
    pub fn insert_trade_chain(
        &self,
        season: SeasonId,
        day: u32,
        state: &NegotiationState,
    ) -> StoreResult<TradeId> {
        let chain_json = serde_json::to_string(state)?;
        let (accepted, final_json) = match state {
            NegotiationState::Accepted(off) => (1_i64, Some(serde_json::to_string(off)?)),
            NegotiationState::Rejected { final_offer, .. } => {
                (0_i64, Some(serde_json::to_string(final_offer)?))
            }
            _ => (0_i64, None),
        };
        self.conn.execute(
            "INSERT INTO trade_history(season, day, accepted, chain_json, final_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![season.0 as i64, day as i64, accepted, chain_json, final_json],
        )?;
        Ok(TradeId(self.conn.last_insert_rowid() as u64))
    }

    /// Update the chain rows without inserting a new id (used after `respond`).
    pub fn update_trade_chain(&self, id: TradeId, state: &NegotiationState) -> StoreResult<()> {
        let chain_json = serde_json::to_string(state)?;
        let (accepted, final_json) = match state {
            NegotiationState::Accepted(off) => (1_i64, Some(serde_json::to_string(off)?)),
            NegotiationState::Rejected { final_offer, .. } => {
                (0_i64, Some(serde_json::to_string(final_offer)?))
            }
            _ => (0_i64, None),
        };
        self.conn.execute(
            "UPDATE trade_history SET accepted = ?2, chain_json = ?3, final_json = ?4 WHERE id = ?1",
            params![id.0 as i64, accepted, chain_json, final_json],
        )?;
        Ok(())
    }

    pub fn read_trade_chain(&self, id: TradeId) -> StoreResult<Option<NegotiationState>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT chain_json FROM trade_history WHERE id = ?1",
                params![id.0 as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    /// Returns (id, state) for every chain in the season ordered by id desc.
    pub fn list_trade_chains(&self, season: SeasonId) -> StoreResult<Vec<(TradeId, NegotiationState)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, chain_json FROM trade_history WHERE season = ?1 ORDER BY id DESC",
        )?;
        let rows = stmt
            .query_map(params![season.0 as i64], |r| {
                let id: i64 = r.get(0)?;
                let chain: String = r.get(1)?;
                Ok((id, chain))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, chain) in rows {
            let state: NegotiationState = serde_json::from_str(&chain)?;
            out.push((TradeId(id as u64), state));
        }
        Ok(out)
    }

    // ------------------------------------------------------------------
    // awards
    // ------------------------------------------------------------------

    /// Insert (or replace) one award winner row. Existing rows for the same
    /// (season, award) key are overwritten — re-running the awards engine is
    /// idempotent.
    pub fn record_award(
        &self,
        season: SeasonId,
        award: &str,
        player: PlayerId,
    ) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO awards(season, award, player_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(season, award) DO UPDATE SET
                player_id = excluded.player_id",
            params![season.0 as i64, award, player.0 as i64],
        )?;
        Ok(())
    }

    /// All awards for `season` ordered by award name.
    pub fn read_awards(&self, season: SeasonId) -> StoreResult<Vec<(String, PlayerId)>> {
        let mut stmt = self.conn.prepare(
            "SELECT award, player_id FROM awards WHERE season = ?1 ORDER BY award ASC",
        )?;
        let rows = stmt
            .query_map(params![season.0 as i64], |r| {
                let award: String = r.get(0)?;
                let pid: i64 = r.get(1)?;
                Ok((award, PlayerId(pid as u32)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // playoff series
    // ------------------------------------------------------------------

    /// Append a series row. Caller is responsible for not double-recording.
    /// Returns the auto-assigned series id.
    pub fn record_series(&self, row: &SeriesRow) -> StoreResult<i64> {
        let games_json = serde_json::to_string(&row.games)?;
        self.conn.execute(
            "INSERT INTO series(season, round, home_team, away_team,
                                home_wins, away_wins, games_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                row.season.0 as i64,
                row.round as i64,
                row.home_team.0 as i64,
                row.away_team.0 as i64,
                row.home_wins as i64,
                row.away_wins as i64,
                games_json,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// All series rows for `season` in (round asc, id asc) order.
    pub fn read_series(&self, season: SeasonId) -> StoreResult<Vec<SeriesRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT season, round, home_team, away_team, home_wins, away_wins, games_json
             FROM series WHERE season = ?1 ORDER BY round ASC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![season.0 as i64], |r| {
                let season: i64 = r.get(0)?;
                let round: i64 = r.get(1)?;
                let home: i64 = r.get(2)?;
                let away: i64 = r.get(3)?;
                let hw: i64 = r.get(4)?;
                let aw: i64 = r.get(5)?;
                let games: String = r.get(6)?;
                Ok((season, round, home, away, hw, aw, games))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (season, round, home, away, hw, aw, games_json) in rows {
            let games: Vec<GameResult> = serde_json::from_str(&games_json)?;
            out.push(SeriesRow {
                season: SeasonId(season as u16),
                round: round as u8,
                home_team: TeamId(home as u8),
                away_team: TeamId(away as u8),
                home_wins: hw as u8,
                away_wins: aw as u8,
                games,
            });
        }
        Ok(out)
    }

    pub fn read_standings(&self, season: SeasonId) -> StoreResult<Vec<StandingRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.team_id, t.abbrev, t.conference, t.division,
                    s.wins, s.losses, s.conf_rank
             FROM standings s JOIN teams t ON t.id = s.team_id
             WHERE s.season = ?1
             ORDER BY s.wins DESC, s.losses ASC, t.abbrev ASC",
        )?;
        let rows = stmt
            .query_map(params![season.0 as i64], |r| {
                let id: i64 = r.get(0)?;
                let abbrev: String = r.get(1)?;
                let conf: String = r.get(2)?;
                let div: String = r.get(3)?;
                let wins: i64 = r.get(4)?;
                let losses: i64 = r.get(5)?;
                let conf_rank: Option<i64> = r.get(6)?;
                Ok(StandingRow {
                    team: TeamId(id as u8),
                    abbrev,
                    conference: parse_conference(&conf),
                    division: parse_division(&div),
                    wins: wins as u16,
                    losses: losses as u16,
                    conf_rank: conf_rank.map(|n| n as u8),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ScheduledRow {
    pub game_id: u64,
    pub season: SeasonId,
    pub date: chrono::NaiveDate,
    pub home: TeamId,
    pub away: TeamId,
}

/// Row representation of a persisted playoff series. The home team is the
/// higher seed (opens at home in the 2-2-1-1-1 schedule).
#[derive(Debug, Clone)]
pub struct SeriesRow {
    pub season: SeasonId,
    pub round: u8,
    pub home_team: TeamId,
    pub away_team: TeamId,
    pub home_wins: u8,
    pub away_wins: u8,
    pub games: Vec<GameResult>,
}

#[derive(Debug, Clone)]
pub struct StandingRow {
    pub team: TeamId,
    pub abbrev: String,
    pub conference: Conference,
    pub division: Division,
    pub wins: u16,
    pub losses: u16,
    pub conf_rank: Option<u8>,
}

type PlayerRow = (
    i64,                  // 0  id
    String,               // 1  name
    String,               // 2  primary_position
    Option<String>,       // 3  secondary_position
    i64,                  // 4  age
    i64,                  // 5  overall
    i64,                  // 6  potential
    String,               // 7  ratings_json
    Option<String>,       // 8  contract_json
    Option<i64>,          // 9  team_id
    Option<String>,       // 10 injury_json
    i64,                  // 11 no_trade_clause
    Option<i64>,          // 12 trade_kicker_pct
);

fn read_player_row(r: &rusqlite::Row) -> rusqlite::Result<PlayerRow> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
        r.get(8)?,
        r.get(9)?,
        r.get(10)?,
        r.get(11)?,
        r.get(12)?,
    ))
}

fn deserialize_player(r: PlayerRow) -> StoreResult<Player> {
    let primary_position = parse_position(&r.2);
    let secondary_position = r.3.as_deref().map(parse_position);
    let ratings: Ratings = serde_json::from_str(&r.7)?;
    let contract: Option<Contract> =
        r.8.as_deref().map(serde_json::from_str).transpose()?;
    let team = r.9.map(|n| TeamId(n as u8));
    let injury: Option<InjuryStatus> =
        r.10.as_deref().map(serde_json::from_str).transpose()?;
    Ok(Player {
        id: PlayerId(r.0 as u32),
        name: r.1,
        primary_position,
        secondary_position,
        age: r.4 as u8,
        overall: r.5 as u8,
        potential: r.6 as u8,
        ratings,
        contract,
        team,
        injury,
        no_trade_clause: r.11 != 0,
        trade_kicker_pct: r.12.map(|n| n as u8),
        role: PlayerRole::default(),
        morale: 0.5,
    })
}

fn parse_position(s: &str) -> Position {
    match s {
        "PG" => Position::PG,
        "SG" => Position::SG,
        "SF" => Position::SF,
        "PF" => Position::PF,
        _ => Position::C,
    }
}

fn parse_conference(s: &str) -> Conference {
    match s {
        "East" => Conference::East,
        _ => Conference::West,
    }
}

fn parse_division(s: &str) -> Division {
    match s {
        "Atlantic" => Division::Atlantic,
        "Central" => Division::Central,
        "Southeast" => Division::Southeast,
        "Northwest" => Division::Northwest,
        "Pacific" => Division::Pacific,
        _ => Division::Southwest,
    }
}
