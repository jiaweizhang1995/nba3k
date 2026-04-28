use crate::StoreResult;
use nba3k_core::*;
use nba3k_models::progression::PlayerDevelopment;
use nba3k_season::career::{aggregate_career, SeasonAvgRow};
use rusqlite::types::ValueRef;
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
    // settings
    // ------------------------------------------------------------------

    pub fn write_setting(&self, key: &str, value: &str) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO settings(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn read_setting(&self, key: &str) -> StoreResult<Option<String>> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v)
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
                injury_json, no_trade_clause, trade_kicker_pct,
                role_str, morale
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
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
                trade_kicker_pct = excluded.trade_kicker_pct,
                role_str = excluded.role_str,
                morale = excluded.morale",
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
                p.role.to_string(),
                p.morale as f64,
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
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)
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
                    trade_kicker_pct = excluded.trade_kicker_pct,
                    role_str = excluded.role_str,
                    morale = excluded.morale",
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
                    p.role.to_string(),
                    p.morale as f64,
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
                injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
             ) VALUES (?1,?2,?3,NULL,?4,?5,?6,?7,NULL,NULL,NULL,0,NULL,'Prospect',0.5)
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

    /// List all prospects (players with no team) ordered best-first by
    /// (potential desc, overall desc, id asc). Result is `Vec<Player>` —
    /// callers map back to `DraftProspect` if they need the full shape.
    pub fn list_prospects(&self) -> StoreResult<Vec<Player>> {
        // Prospects vs free agents are disjoint pools post-V006: prospects
        // keep is_free_agent = 0 so the FA pool never leaks into the draft
        // board.
        let mut stmt = self.conn.prepare(
            "SELECT id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
             FROM players WHERE team_id IS NULL AND is_free_agent = 0
             ORDER BY potential DESC, overall DESC, id ASC",
        )?;
        let rows = stmt
            .query_map([], read_player_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(deserialize_player).collect()
    }

    /// Assign a prospect (or any player with no team) to a team. Used by
    /// the draft pipeline to convert prospects → roster players, and by the
    /// FA pool when signing free agents. Clears `is_free_agent` so a signed
    /// FA stops appearing in the FA pool.
    pub fn assign_player_to_team(&self, player_id: PlayerId, team: TeamId) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE players SET team_id = ?1, is_free_agent = 0 WHERE id = ?2",
            params![team.0 as i64, player_id.0 as i64],
        )?;
        Ok(())
    }

    pub fn count_prospects(&self) -> StoreResult<u32> {
        // Prospects = unsigned players that are NOT in the free-agent pool.
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM players
             WHERE team_id IS NULL AND is_free_agent = 0",
            [],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    // ------------------------------------------------------------------
    // scouting fog (M14)
    //
    // `scouted` defaults to 0. The CLI hides un-scouted prospects' OVR,
    // POT, and full Ratings; `cmd_scout` flips the flag to 1 to reveal.
    // The store keeps the truthful values regardless — fog is a render
    // concern, not a data concern.
    // ------------------------------------------------------------------

    /// Flip the `scouted` flag on a single player. Idempotent.
    pub fn set_player_scouted(&self, player_id: PlayerId, scouted: bool) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE players SET scouted = ?1 WHERE id = ?2",
            params![if scouted { 1_i64 } else { 0 }, player_id.0 as i64],
        )?;
        Ok(())
    }

    /// Read the `scouted` flag for one player. Returns `Ok(false)` when the
    /// row doesn't exist — the caller's existing player-lookup will surface
    /// that case explicitly.
    pub fn is_player_scouted(&self, player_id: PlayerId) -> StoreResult<bool> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT scouted FROM players WHERE id = ?1",
                params![player_id.0 as i64],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0) != 0)
    }

    /// List prospects paired with their `scouted` flag. Sorted so the
    /// scouted prospects bubble to the top by potential desc, with the
    /// un-scouted tail ordered alphabetically — matches the draft-board
    /// fog rules in `cmd_draft_board`.
    pub fn list_prospects_visible(&self) -> StoreResult<Vec<(Player, bool)>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale,
                    scouted
             FROM players
             WHERE team_id IS NULL AND is_free_agent = 0 AND is_retired = 0
             ORDER BY scouted DESC,
                      CASE WHEN scouted = 1 THEN -potential ELSE 0 END ASC,
                      CASE WHEN scouted = 1 THEN -overall   ELSE 0 END ASC,
                      CASE WHEN scouted = 0 THEN lower(name) END ASC,
                      id ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let row = read_player_row(r)?;
                let scouted: i64 = r.get(15)?;
                Ok((row, scouted != 0))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(row, scouted)| Ok((deserialize_player(row)?, scouted)))
            .collect()
    }

    // ------------------------------------------------------------------
    // free-agent pool (M10)
    //
    // A player is a free agent when `team_id IS NULL AND is_free_agent = 1`.
    // Prospects keep `is_free_agent = 0`, so the two pools never overlap.
    // ------------------------------------------------------------------

    /// List all free agents ordered best-first by overall (then potential,
    /// then id, for deterministic tiebreak).
    pub fn list_free_agents(&self) -> StoreResult<Vec<Player>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
             FROM players
             WHERE team_id IS NULL AND is_free_agent = 1
             ORDER BY overall DESC, potential DESC, id ASC",
        )?;
        let rows = stmt
            .query_map([], read_player_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(deserialize_player).collect()
    }

    /// Cut a player from their team into the free-agent pool. Clears the
    /// team assignment and flips `is_free_agent = 1`.
    pub fn cut_player(&self, player_id: PlayerId) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE players
             SET team_id = NULL, is_free_agent = 1
             WHERE id = ?1",
            params![player_id.0 as i64],
        )?;
        Ok(())
    }

    pub fn count_free_agents(&self) -> StoreResult<u32> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM players
             WHERE team_id IS NULL AND is_free_agent = 1",
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
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
             FROM players
             WHERE team_id = ?1 AND is_retired = 0
             ORDER BY overall DESC, id ASC",
        )?;
        let rows = stmt
            .query_map(params![team.0 as i64], read_player_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(deserialize_player).collect()
    }

    /// Mark `player_id` as retired. Clears the team assignment and the
    /// free-agent flag so retirees never appear in active rosters, the FA
    /// pool, or the prospect pool. Idempotent — re-retiring is a no-op.
    pub fn set_player_retired(&self, player_id: PlayerId) -> StoreResult<()> {
        self.conn.execute(
            "UPDATE players
             SET team_id = NULL, is_free_agent = 0, is_retired = 1
             WHERE id = ?1",
            params![player_id.0 as i64],
        )?;
        Ok(())
    }

    /// All retired players, ordered by overall desc — for future HOF UI.
    pub fn list_retired_players(&self) -> StoreResult<Vec<Player>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, primary_position, secondary_position, age,
                    overall, potential, ratings_json, contract_json, team_id,
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
             FROM players
             WHERE is_retired = 1
             ORDER BY overall DESC, id ASC",
        )?;
        let rows = stmt
            .query_map([], read_player_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter().map(deserialize_player).collect()
    }

    /// Sum of every roster player's contract salary for `season`.
    /// Players with no contract or no matching season-year contribute zero —
    /// matches NBA cap-hold accounting where empty roster slots are not a
    /// liability. Use `LeagueYear::for_season` to compare against cap/tax/aprons.
    pub fn team_salary(&self, team: TeamId, season: SeasonId) -> StoreResult<Cents> {
        let roster = self.roster_for_team(team)?;
        let total = roster
            .iter()
            .map(|p| {
                p.contract
                    .as_ref()
                    .map(|c| c.salary_for(season))
                    .unwrap_or(Cents::ZERO)
            })
            .sum();
        Ok(total)
    }

    pub fn find_player_by_name(&self, name: &str) -> StoreResult<Option<Player>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, name, primary_position, secondary_position, age,
                        overall, potential, ratings_json, contract_json, team_id,
                        injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
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

    /// Wipe schedule rows for `season`. Used by season-advance to drop
    /// last year's slate before regenerating fresh games. The historical
    /// `games` table is untouched so prior-season records remain available.
    pub fn clear_schedule_for_season(&self, season: SeasonId) -> StoreResult<()> {
        self.conn.execute(
            "DELETE FROM schedule WHERE season = ?1",
            params![season.0 as i64],
        )?;
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

    /// Distinct seasons that have at least one row in `games` (any phase).
    /// Returned ascending. Used by career-stats walks that span the whole save.
    pub fn distinct_game_seasons(&self) -> StoreResult<Vec<SeasonId>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT season FROM games ORDER BY season ASC")?;
        let rows = stmt
            .query_map([], |r| {
                let s: i64 = r.get(0)?;
                Ok(SeasonId(s as u16))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Aggregate career stats for one player across every season recorded in
    /// `games`. Empty Vec when the player never appears in any box score.
    /// Per-season rows are ordered by `SeasonId` ascending — same order as
    /// `distinct_game_seasons`.
    pub fn read_career_stats(&self, player: PlayerId) -> StoreResult<Vec<SeasonAvgRow>> {
        let mut out: Vec<SeasonAvgRow> = Vec::new();
        for season in self.distinct_game_seasons()? {
            let games = self.read_games(season)?;
            out.extend(aggregate_career(&games, player));
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
                    injury_json, no_trade_clause, trade_kicker_pct, role_str, morale
             FROM players WHERE team_id IS NOT NULL AND is_retired = 0",
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

    /// Open chains for `season` where `team` is a participant but is NOT the
    /// initiator — i.e. "incoming" offers from `team`'s perspective. Used by
    /// M17 `offers` to surface AI proposals to the user.
    pub fn read_open_chains_targeting(
        &self,
        season: SeasonId,
        team: TeamId,
    ) -> StoreResult<Vec<(TradeId, NegotiationState)>> {
        let chains = self.list_trade_chains(season)?;
        let mut out = Vec::new();
        for (id, state) in chains {
            let NegotiationState::Open { ref chain } = state else { continue };
            let Some(latest) = chain.last() else { continue };
            if latest.initiator == team {
                continue;
            }
            if !latest.assets_by_team.contains_key(&team) {
                continue;
            }
            out.push((id, state));
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
    // all-star (M15-A)
    // ------------------------------------------------------------------

    /// Insert one All-Star roster row. Idempotent on (season, player_id) —
    /// re-running the day-41 trigger overwrites the same player's row instead
    /// of duplicating it.
    pub fn record_all_star(
        &self,
        season: SeasonId,
        conf: Conference,
        player: PlayerId,
        role: &str,
    ) -> StoreResult<()> {
        let conf_tag = match conf {
            Conference::East => "East",
            Conference::West => "West",
        };
        self.conn.execute(
            "INSERT INTO all_star(season, conf, player_id, role) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(season, player_id) DO UPDATE SET
                conf = excluded.conf,
                role = excluded.role",
            params![season.0 as i64, conf_tag, player.0 as i64, role],
        )?;
        Ok(())
    }

    /// All-Star rows for `season` as `(conference, role, player_id)`. Ordered
    /// by conference asc then role asc so callers get a stable display order.
    pub fn read_all_star(
        &self,
        season: SeasonId,
    ) -> StoreResult<Vec<(Conference, String, PlayerId)>> {
        let mut stmt = self.conn.prepare(
            "SELECT conf, role, player_id FROM all_star
             WHERE season = ?1
             ORDER BY conf ASC, role ASC, player_id ASC",
        )?;
        let rows = stmt
            .query_map(params![season.0 as i64], |r| {
                let conf: String = r.get(0)?;
                let role: String = r.get(1)?;
                let pid: i64 = r.get(2)?;
                Ok((conf, role, PlayerId(pid as u32)))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let out = rows
            .into_iter()
            .map(|(conf, role, pid)| {
                let c = if conf == "West" { Conference::West } else { Conference::East };
                (c, role, pid)
            })
            .collect();
        Ok(out)
    }

    // ------------------------------------------------------------------
    // news feed (M13)
    // ------------------------------------------------------------------

    /// Append one news row. `kind` is a free-form tag (e.g. "trade", "signing",
    /// "cut", "retire", "draft", "award", "injury"); callers pick what fits.
    pub fn record_news(
        &self,
        season: SeasonId,
        day: u32,
        kind: &str,
        headline: &str,
        body: Option<&str>,
    ) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO news(season, day, kind, headline, body)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![season.0 as i64, day as i64, kind, headline, body],
        )?;
        Ok(())
    }

    /// Most recent N news rows, newest first (by insertion id).
    pub fn recent_news(&self, limit: u32) -> StoreResult<Vec<NewsRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT season, day, kind, headline, body
             FROM news
             ORDER BY id DESC
             LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit as i64], |r| {
                let season: i64 = r.get(0)?;
                let day: i64 = r.get(1)?;
                let kind: String = r.get(2)?;
                let headline: String = r.get(3)?;
                let body: Option<String> = r.get(4)?;
                Ok(NewsRow {
                    season: SeasonId(season as u16),
                    day: day as u32,
                    kind,
                    headline,
                    body,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // NBA Cup (M16-A)
    //
    // Cup matches live in their own table so they don't pollute `standings`
    // or the regular-season box-score readers. Group rows carry a non-NULL
    // `group_id` ("east-A".."west-C"); KO rows leave it NULL.
    // ------------------------------------------------------------------

    /// Append one cup match. `round` is "group" | "qf" | "sf" | "final";
    /// `group_id` should be `Some("east-A")` etc. for group rows and `None`
    /// for KO rounds.
    #[allow(clippy::too_many_arguments)]
    pub fn record_cup_match(
        &self,
        season: SeasonId,
        round: &str,
        group_id: Option<&str>,
        home: TeamId,
        away: TeamId,
        home_score: u16,
        away_score: u16,
        day: u32,
    ) -> StoreResult<()> {
        self.conn.execute(
            "INSERT INTO cup_match(season, round, group_id, home_team, away_team,
                                   home_score, away_score, day)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                season.0 as i64,
                round,
                group_id,
                home.0 as i64,
                away.0 as i64,
                home_score as i64,
                away_score as i64,
                day as i64,
            ],
        )?;
        Ok(())
    }

    /// All cup rows for `season` ordered by id ascending — preserves the
    /// (round → match) insertion order the trigger produced.
    pub fn read_cup_matches(&self, season: SeasonId) -> StoreResult<Vec<CupMatchRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT season, round, group_id, home_team, away_team,
                    home_score, away_score, day
             FROM cup_match WHERE season = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt
            .query_map(params![season.0 as i64], |r| {
                let season: i64 = r.get(0)?;
                let round: String = r.get(1)?;
                let group_id: Option<String> = r.get(2)?;
                let home: i64 = r.get(3)?;
                let away: i64 = r.get(4)?;
                let hs: i64 = r.get(5)?;
                let aw: i64 = r.get(6)?;
                let day: i64 = r.get(7)?;
                Ok(CupMatchRow {
                    season: SeasonId(season as u16),
                    round,
                    group_id,
                    home_team: TeamId(home as u8),
                    away_team: TeamId(away as u8),
                    home_score: hs as u16,
                    away_score: aw as u16,
                    day: day as u32,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    // ------------------------------------------------------------------
    // notes / favorites (M17-C)
    //
    // One row per player; UPSERT so re-adding a note replaces the text.
    // `created_at` is stamped on the most recent write.
    // ------------------------------------------------------------------

    /// Upsert a note for `player_id`. Re-inserting overwrites the text
    /// and refreshes `created_at`.
    pub fn insert_note(&self, player_id: PlayerId, text: &str) -> StoreResult<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO notes(player_id, text, created_at)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(player_id) DO UPDATE SET
                text = excluded.text,
                created_at = excluded.created_at",
            params![player_id.0 as i64, text, now],
        )?;
        Ok(())
    }

    /// Delete the note for `player_id`. No-op if no note exists; returns
    /// the number of rows removed (0 or 1).
    pub fn delete_note(&self, player_id: PlayerId) -> StoreResult<usize> {
        let n = self.conn.execute(
            "DELETE FROM notes WHERE player_id = ?1",
            params![player_id.0 as i64],
        )?;
        Ok(n)
    }

    /// All notes ordered by `created_at` ascending (oldest first).
    pub fn list_notes(&self) -> StoreResult<Vec<NoteRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT player_id, text, created_at
             FROM notes
             ORDER BY created_at ASC, player_id ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let pid: i64 = r.get(0)?;
                let text: Option<String> = r.get(1)?;
                let created_at: String = r.get(2)?;
                Ok(NoteRow {
                    player_id: PlayerId(pid as u32),
                    text,
                    created_at,
                })
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

    // ------------------------------------------------------------------
    // rotation level A — user-set starters per team (M21, V014)
    //
    // `team_starters` stores at most 5 rows per team, one per canonical
    // position string. The CHECK constraint guards SQL-side; we still
    // validate `pos` at API entry so callers fail fast with a clear
    // error rather than a sqlite constraint violation.
    // ------------------------------------------------------------------

    /// Read the user-set starters for a team. Empty / partial overrides
    /// return as `Default` slots — callers use `Starters::is_complete`
    /// to decide whether to honor the override.
    pub fn read_starters(&self, team_id: TeamId) -> StoreResult<nba3k_core::Starters> {
        let mut stmt = self
            .conn
            .prepare("SELECT pos, player_id FROM team_starters WHERE team_id = ?1")?;
        let rows = stmt
            .query_map(params![team_id.0 as i64], |r| {
                let pos: String = r.get(0)?;
                let pid: i64 = r.get(1)?;
                Ok((pos, pid))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut starters = nba3k_core::Starters::default();
        for (pos, pid) in rows {
            let player = PlayerId(pid as u32);
            match pos.as_str() {
                "PG" => starters.pg = Some(player),
                "SG" => starters.sg = Some(player),
                "SF" => starters.sf = Some(player),
                "PF" => starters.pf = Some(player),
                "C" => starters.c = Some(player),
                // Unknown rows can't exist thanks to the CHECK constraint,
                // but we treat them as no-ops rather than panicking — the
                // sim hook gracefully falls back when slots are missing.
                _ => {}
            }
        }
        Ok(starters)
    }

    /// Set or replace the starter at one positional slot.
    pub fn upsert_starter(
        &self,
        team_id: TeamId,
        pos: &str,
        player_id: PlayerId,
    ) -> StoreResult<()> {
        validate_starter_pos(pos)?;
        self.conn.execute(
            "INSERT INTO team_starters(team_id, pos, player_id)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(team_id, pos) DO UPDATE SET player_id = excluded.player_id",
            params![team_id.0 as i64, pos, player_id.0 as i64],
        )?;
        Ok(())
    }

    /// Clear one positional slot. No-op if the slot was already empty.
    pub fn clear_starter(&self, team_id: TeamId, pos: &str) -> StoreResult<()> {
        validate_starter_pos(pos)?;
        self.conn.execute(
            "DELETE FROM team_starters WHERE team_id = ?1 AND pos = ?2",
            params![team_id.0 as i64, pos],
        )?;
        Ok(())
    }

    /// Wipe all five slots for a team. Used by the "Clear all" UI action.
    pub fn clear_all_starters(&self, team_id: TeamId) -> StoreResult<()> {
        self.conn.execute(
            "DELETE FROM team_starters WHERE team_id = ?1",
            params![team_id.0 as i64],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // export (M18-C)
    //
    // Dump every persistent user table as `{tables: {name: [{col: val,
    // ...}, ...], ...}}`. Schema is read from `sqlite_master` so new
    // migrations are picked up automatically. Refinery's bookkeeping
    // table and SQLite internals are skipped.
    // ------------------------------------------------------------------

    pub fn dump_to_json(&self) -> StoreResult<serde_json::Value> {
        let table_names: Vec<String> = {
            let mut stmt = self.conn.prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table'
                   AND name NOT LIKE 'sqlite_%'
                   AND name NOT LIKE '_refinery%'
                   AND name NOT LIKE 'refinery_%'
                 ORDER BY name ASC",
            )?;
            let names = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            names
        };

        let mut tables = serde_json::Map::with_capacity(table_names.len());
        for name in table_names {
            let rows = self.dump_table_rows(&name)?;
            tables.insert(name, serde_json::Value::Array(rows));
        }
        Ok(serde_json::json!({ "tables": tables }))
    }

    fn dump_table_rows(&self, table: &str) -> StoreResult<Vec<serde_json::Value>> {
        // SQLite identifier quoting: double the embedded quotes. Table
        // names come from sqlite_master so they're already valid, but we
        // quote defensively to handle any reserved words.
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let sql = format!("SELECT * FROM {}", quoted);
        let mut stmt = self.conn.prepare(&sql)?;
        let col_names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let col_count = col_names.len();
        let mut rows_iter = stmt.query([])?;
        let mut out: Vec<serde_json::Value> = Vec::new();
        while let Some(row) = rows_iter.next()? {
            let mut obj = serde_json::Map::with_capacity(col_count);
            for (i, col) in col_names.iter().enumerate() {
                let v = row.get_ref(i)?;
                obj.insert(col.clone(), value_ref_to_json(v));
            }
            out.push(serde_json::Value::Object(obj));
        }
        Ok(out)
    }
}

fn value_ref_to_json(v: ValueRef<'_>) -> serde_json::Value {
    match v {
        ValueRef::Null => serde_json::Value::Null,
        ValueRef::Integer(n) => serde_json::Value::Number(n.into()),
        ValueRef::Real(f) => serde_json::Number::from_f64(f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        ValueRef::Text(bytes) => match std::str::from_utf8(bytes) {
            Ok(s) => serde_json::Value::String(s.to_string()),
            Err(_) => serde_json::Value::String(String::from_utf8_lossy(bytes).into_owned()),
        },
        ValueRef::Blob(bytes) => {
            // Blobs become arrays of byte values. Keeps the dump
            // round-trippable without dragging base64 in.
            serde_json::Value::Array(
                bytes
                    .iter()
                    .map(|b| serde_json::Value::Number((*b as u64).into()))
                    .collect(),
            )
        }
    }
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// Reject any `pos` that doesn't match the canonical 5-position string.
/// Mirrors the V014 `CHECK(pos IN ('PG','SG','SF','PF','C'))` so callers
/// fail fast with a typed error instead of an opaque sqlite constraint
/// violation.
fn validate_starter_pos(pos: &str) -> StoreResult<()> {
    match pos {
        "PG" | "SG" | "SF" | "PF" | "C" => Ok(()),
        _ => Err(crate::StoreError::InvalidInput(format!(
            "invalid starter position: {pos:?} (expected PG|SG|SF|PF|C)"
        ))),
    }
}

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

#[derive(Debug, Clone)]
pub struct NewsRow {
    pub season: SeasonId,
    pub day: u32,
    pub kind: String,
    pub headline: String,
    pub body: Option<String>,
}

/// Row representation of one NBA Cup match (M16-A). `round` is one of
/// "group" | "qf" | "sf" | "final"; `group_id` is `Some("east-A".."west-C")`
/// for group rows, `None` for KO rounds.
#[derive(Debug, Clone)]
pub struct CupMatchRow {
    pub season: SeasonId,
    pub round: String,
    pub group_id: Option<String>,
    pub home_team: TeamId,
    pub away_team: TeamId,
    pub home_score: u16,
    pub away_score: u16,
    pub day: u32,
}

/// Row representation of one player note (M17-C). `text` is optional so
/// callers can flag a player without leaving prose; `created_at` is
/// stamped on the most recent write.
#[derive(Debug, Clone)]
pub struct NoteRow {
    pub player_id: PlayerId,
    pub text: Option<String>,
    pub created_at: String,
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
    String,               // 13 role_str
    f64,                  // 14 morale
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
        r.get(13)?,
        r.get(14)?,
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
        role: parse_role(&r.13),
        morale: r.14 as f32,
    })
}

fn parse_role(s: &str) -> PlayerRole {
    match s {
        "Star" => PlayerRole::Star,
        "Starter" => PlayerRole::Starter,
        "SixthMan" => PlayerRole::SixthMan,
        "BenchWarmer" => PlayerRole::BenchWarmer,
        "Prospect" => PlayerRole::Prospect,
        _ => PlayerRole::RolePlayer,
    }
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
