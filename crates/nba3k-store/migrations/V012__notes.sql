-- M17-C player notes / favorites. Lightweight per-player tracker the GM
-- can use to flag players to revisit. One row per player; UPSERT
-- semantics so re-adding a note replaces the text.

CREATE TABLE notes (
    player_id INTEGER PRIMARY KEY REFERENCES players(id),
    text TEXT,
    created_at TEXT NOT NULL
);
