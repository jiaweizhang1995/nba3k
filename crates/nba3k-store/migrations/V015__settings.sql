-- M23 Settings. Small key/value table for user preferences such as TUI
-- language. Keys are stable ASCII identifiers; values are stored as text.

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
