-- M5 role + morale persistence. NBA 2K's MyGM uses role tags (Star /
-- Starter / SixthMan / RolePlayer / BenchWarmer / Prospect) to drive
-- chemistry + morale-shift events. `morale` is a 0..=1 float; default
-- 0.5 means "neutral" so legacy players read back as before.

ALTER TABLE players ADD COLUMN role_str TEXT NOT NULL DEFAULT 'RolePlayer';
ALTER TABLE players ADD COLUMN morale REAL NOT NULL DEFAULT 0.5;
