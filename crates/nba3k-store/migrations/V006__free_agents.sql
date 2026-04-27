-- M10 free-agency v2: distinguish prospects from free agents.
--
-- Prior to V006 the only `team_id IS NULL` players were draft prospects.
-- Cutting a player now also leaves `team_id = NULL` but with
-- `is_free_agent = 1`, so `list_free_agents` and `list_prospects` can be
-- queried independently. Default 0 keeps all existing rows (prospects
-- + rostered players) classified as non-FAs.

ALTER TABLE players ADD COLUMN is_free_agent INTEGER NOT NULL DEFAULT 0;

CREATE INDEX idx_players_free_agent ON players(is_free_agent) WHERE is_free_agent = 1;
