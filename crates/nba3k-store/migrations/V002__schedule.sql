-- Persistent schedule for the active season. Rows are inserted once on
-- `nba3k new` and consumed (marked `played = 1`) during `sim-day` / `sim-to`.
-- Played game results live in the existing `games` table.

CREATE TABLE schedule (
    game_id   INTEGER PRIMARY KEY NOT NULL,
    season    INTEGER NOT NULL,
    date      TEXT NOT NULL,
    home      INTEGER NOT NULL REFERENCES teams(id),
    away      INTEGER NOT NULL REFERENCES teams(id),
    played    INTEGER NOT NULL DEFAULT 0,
    is_playoffs INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_schedule_date     ON schedule(date);
CREATE INDEX idx_schedule_season_played ON schedule(season, played);
CREATE INDEX idx_schedule_home_date ON schedule(home, date);
CREATE INDEX idx_schedule_away_date ON schedule(away, date);
