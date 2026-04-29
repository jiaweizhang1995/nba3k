-- M36: make draft-pick ownership durable enough for traded picks.

ALTER TABLE draft_picks ADD COLUMN resolved INTEGER NOT NULL DEFAULT 0;
ALTER TABLE draft_picks ADD COLUMN protection_text TEXT DEFAULT NULL;
ALTER TABLE draft_picks ADD COLUMN protection_history TEXT DEFAULT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS idx_picks_original_season_round
    ON draft_picks(season, original_team, round);

CREATE INDEX IF NOT EXISTS idx_picks_original_season
    ON draft_picks(original_team, season);
