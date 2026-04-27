-- M15-A All-Star roster: persisted at the day-41 mid-season marker so the
-- past selections survive across season-advance and remain queryable per
-- season. `conf` is "East" | "West", `role` is "starter" | "reserve" — both
-- string-tagged for stable JSON rendering without enum churn.

CREATE TABLE all_star (
    season INTEGER NOT NULL,
    conf TEXT NOT NULL,
    player_id INTEGER NOT NULL REFERENCES players(id),
    role TEXT NOT NULL,
    PRIMARY KEY (season, player_id)
);
CREATE INDEX idx_all_star_season ON all_star(season);
