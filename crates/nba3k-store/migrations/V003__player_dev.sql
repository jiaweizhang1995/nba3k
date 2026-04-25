-- M5 progression engine: per-player development state.
--
-- `dev_json` carries the JSON-serialized `PlayerDevelopment` (peak window,
-- dynamic_potential, work_ethic, last_progressed_season). NULL means the
-- player has not yet been initialized — the Store read API auto-bootstraps
-- with defaults at first read.

ALTER TABLE players ADD COLUMN dev_json TEXT;
