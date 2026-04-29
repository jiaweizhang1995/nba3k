-- M31 Season calendar. Per-season window driving schedule + phase math.
-- Replaces hardcoded SEASON_START / SEASON_END / trade-deadline constants.
-- The 2026 row is seeded so existing 2025-26 saves keep working unchanged.

CREATE TABLE season_calendar (
    season_year     INTEGER PRIMARY KEY,
    start_date      TEXT NOT NULL,
    end_date        TEXT NOT NULL,
    trade_deadline  TEXT NOT NULL,
    all_star_day    INTEGER NOT NULL DEFAULT 41,
    cup_group_day   INTEGER NOT NULL DEFAULT 30,
    cup_qf_day      INTEGER NOT NULL DEFAULT 45,
    cup_sf_day      INTEGER NOT NULL DEFAULT 53,
    cup_final_day   INTEGER NOT NULL DEFAULT 55
);

INSERT INTO season_calendar (season_year, start_date, end_date, trade_deadline)
VALUES (2026, '2025-10-21', '2026-04-12', '2026-02-05');
