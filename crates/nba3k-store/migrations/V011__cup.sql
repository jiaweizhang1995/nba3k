-- M16-A NBA Cup: in-season tournament parallel to the regular season.
-- Cup matches are persisted independently from `games` so they never leak
-- into standings or the regular-season box-score readers. `round` is one
-- of "group" | "qf" | "sf" | "final"; `group_id` is "east-A".."west-C" for
-- group rows and NULL for KO rounds.

CREATE TABLE cup_match (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    season INTEGER NOT NULL,
    round TEXT NOT NULL,
    group_id TEXT,
    home_team INTEGER NOT NULL,
    away_team INTEGER NOT NULL,
    home_score INTEGER NOT NULL,
    away_score INTEGER NOT NULL,
    day INTEGER NOT NULL
);
CREATE INDEX idx_cup_season_round ON cup_match(season, round);
