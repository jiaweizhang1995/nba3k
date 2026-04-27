-- M14 scouting fog: prospects' overall/potential/ratings are hidden until
-- the user scouts them. Default 0 = un-scouted. Real NBA players (team_id
-- IS NOT NULL) keep `scouted = 0` but the gate only matters for prospects.

ALTER TABLE players ADD COLUMN scouted INTEGER NOT NULL DEFAULT 0;
