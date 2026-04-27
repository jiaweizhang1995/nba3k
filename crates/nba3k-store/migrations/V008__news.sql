-- M13 news feed: append-only log of league events surfaced via `news`.
--
-- Every state-mutating event (trade accepted, FA signed/cut, retirement,
-- draft summary, award winner) appends one row. `(season, day)` index
-- backs the recent-news query path (ORDER BY id DESC LIMIT N is enough
-- in practice, but the index makes per-season filtering cheap if needed).

CREATE TABLE news (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    season INTEGER NOT NULL,
    day INTEGER NOT NULL,
    kind TEXT NOT NULL,
    headline TEXT NOT NULL,
    body TEXT
);
CREATE INDEX idx_news_season_day ON news(season, day);
