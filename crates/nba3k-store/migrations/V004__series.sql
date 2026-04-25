-- Playoff series persistence. Each row is one best-of-7 matchup.
--
-- `round` 1..=4 maps to: 1=R1, 2=Conf Semifinals, 3=Conf Finals, 4=NBA Finals.
-- `home_team` is the higher-seed team that opens the series at home (2-2-1-1-1).
-- `games_json` is a JSON array of the underlying GameResult rows for replay/UI;
-- the games themselves are also persisted in the `games` table with
-- `is_playoffs = 1`.

CREATE TABLE series (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    season      INTEGER NOT NULL,
    round       INTEGER NOT NULL,
    home_team   INTEGER NOT NULL REFERENCES teams(id),
    away_team   INTEGER NOT NULL REFERENCES teams(id),
    home_wins   INTEGER NOT NULL,
    away_wins   INTEGER NOT NULL,
    games_json  TEXT NOT NULL
);

CREATE INDEX idx_series_season_round ON series(season, round);
