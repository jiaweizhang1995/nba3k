-- M21 Rotation Level A. User-set starting 5 per team. Composite key
-- (team_id, pos) gives upsert semantics; CHECK constraint guards the
-- five canonical position strings. Bench/minutes remain auto.

CREATE TABLE team_starters (
    team_id   INTEGER NOT NULL,
    pos       TEXT    NOT NULL CHECK(pos IN ('PG','SG','SF','PF','C')),
    player_id INTEGER NOT NULL,
    PRIMARY KEY (team_id, pos)
);

CREATE INDEX idx_team_starters_player ON team_starters(player_id);
