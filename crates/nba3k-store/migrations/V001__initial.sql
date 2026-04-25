-- nba3k-claude initial schema

CREATE TABLE meta (
    key   TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);

CREATE TABLE teams (
    id        INTEGER PRIMARY KEY NOT NULL,
    abbrev    TEXT NOT NULL UNIQUE,
    city      TEXT NOT NULL,
    name      TEXT NOT NULL,
    conference TEXT NOT NULL,
    division  TEXT NOT NULL,
    gm_json   TEXT NOT NULL  -- serialized GMPersonality
);

CREATE TABLE players (
    id                 INTEGER PRIMARY KEY NOT NULL,
    name               TEXT NOT NULL,
    primary_position   TEXT NOT NULL,
    secondary_position TEXT,
    age                INTEGER NOT NULL,
    overall            INTEGER NOT NULL,
    potential          INTEGER NOT NULL,
    ratings_json       TEXT NOT NULL,
    contract_json      TEXT,
    team_id            INTEGER REFERENCES teams(id),
    injury_json        TEXT,
    no_trade_clause    INTEGER NOT NULL DEFAULT 0,
    trade_kicker_pct   INTEGER
);

CREATE INDEX idx_players_team ON players(team_id);

CREATE TABLE draft_picks (
    id              INTEGER PRIMARY KEY NOT NULL,
    original_team   INTEGER NOT NULL REFERENCES teams(id),
    current_owner   INTEGER NOT NULL REFERENCES teams(id),
    season          INTEGER NOT NULL,
    round           INTEGER NOT NULL,
    protections_json TEXT NOT NULL
);

CREATE INDEX idx_picks_owner_season ON draft_picks(current_owner, season);

CREATE TABLE games (
    id            INTEGER PRIMARY KEY NOT NULL,
    season        INTEGER NOT NULL,
    date          TEXT NOT NULL,
    home          INTEGER NOT NULL REFERENCES teams(id),
    away          INTEGER NOT NULL REFERENCES teams(id),
    home_score    INTEGER NOT NULL,
    away_score    INTEGER NOT NULL,
    overtime_periods INTEGER NOT NULL DEFAULT 0,
    is_playoffs   INTEGER NOT NULL DEFAULT 0,
    box_score_json TEXT NOT NULL
);

CREATE INDEX idx_games_season ON games(season);
CREATE INDEX idx_games_date ON games(date);

CREATE TABLE trade_history (
    id           INTEGER PRIMARY KEY NOT NULL,
    season       INTEGER NOT NULL,
    day          INTEGER NOT NULL,
    accepted     INTEGER NOT NULL,
    chain_json   TEXT NOT NULL,
    final_json   TEXT
);

CREATE INDEX idx_trades_season_day ON trade_history(season, day);

CREATE TABLE standings (
    team_id    INTEGER NOT NULL REFERENCES teams(id),
    season     INTEGER NOT NULL,
    wins       INTEGER NOT NULL DEFAULT 0,
    losses     INTEGER NOT NULL DEFAULT 0,
    conf_rank  INTEGER,
    PRIMARY KEY (team_id, season)
);

CREATE TABLE awards (
    season    INTEGER NOT NULL,
    award     TEXT NOT NULL,
    player_id INTEGER NOT NULL REFERENCES players(id),
    PRIMARY KEY (season, award)
);

-- Singleton row enforced by check constraint
CREATE TABLE season_state (
    singleton  INTEGER PRIMARY KEY CHECK (singleton = 0),
    state_json TEXT NOT NULL
);
