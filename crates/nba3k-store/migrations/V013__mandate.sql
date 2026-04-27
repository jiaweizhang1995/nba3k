-- M18-A owner mandate. Three season goals are auto-generated at season
-- start (cmd_new + cmd_season_advance) keyed by team strength; progress +
-- weighted grade are rendered by `nba3k mandate`. Composite primary key
-- (season, team, kind) gives upsert semantics so re-running auto-gen is a
-- no-op rather than producing duplicates.

CREATE TABLE mandate (
    season INTEGER NOT NULL,
    team INTEGER NOT NULL,
    kind TEXT NOT NULL,
    target INTEGER NOT NULL,
    weight REAL NOT NULL,
    PRIMARY KEY (season, team, kind)
);
