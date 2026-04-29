-- M32 Player season-to-date aggregates. Populated by the live importer
-- (`build_today_save`) so a fresh "Start From Today" save reflects this
-- season's PPG / RPG / etc. without needing per-game box scores for past
-- games. The table is keyed by (player_id, season_year) so multi-season
-- saves can keep history.

CREATE TABLE player_season_stats (
    player_id   INTEGER NOT NULL,
    season_year INTEGER NOT NULL,
    gp          INTEGER NOT NULL DEFAULT 0,
    mpg         REAL    NOT NULL DEFAULT 0,
    ppg         REAL    NOT NULL DEFAULT 0,
    rpg         REAL    NOT NULL DEFAULT 0,
    apg         REAL    NOT NULL DEFAULT 0,
    spg         REAL    NOT NULL DEFAULT 0,
    bpg         REAL    NOT NULL DEFAULT 0,
    fg_pct      REAL    NOT NULL DEFAULT 0,
    three_pct   REAL    NOT NULL DEFAULT 0,
    ft_pct      REAL    NOT NULL DEFAULT 0,
    ts_pct      REAL    NOT NULL DEFAULT 0,
    usage       REAL    NOT NULL DEFAULT 0,
    PRIMARY KEY (player_id, season_year),
    FOREIGN KEY (player_id) REFERENCES players(id) ON DELETE CASCADE
);

CREATE INDEX idx_pss_season ON player_season_stats(season_year);
