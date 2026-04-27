-- M11 retirement engine: aging players hang it up at season end.
--
-- Retired players have `team_id = NULL` (off-roster) AND `is_retired = 1`,
-- which keeps them off active queries (`roster_for_team`, `all_active_players`)
-- and out of the FA / prospect pools (both gate on `is_retired = 0`).
-- Default 0 keeps every existing row classified as active.

ALTER TABLE players ADD COLUMN is_retired INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_players_retired ON players(is_retired) WHERE is_retired = 1;
