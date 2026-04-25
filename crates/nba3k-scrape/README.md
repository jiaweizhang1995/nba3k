# nba3k-scrape

Produces `data/seed_<season>.sqlite` — the canonical seed database read by
`nba3k-cli new`. One-shot bootstrap from public NBA sources.

## Usage

```bash
cargo run -p nba3k-scrape -- --season 2025-26 --out data/seed_2025_26.sqlite
```

Flags:

- `--season <YYYY-YY>` — defaults to `2025-26`. Must be encoded in
  `nba3k-core::LEAGUE_YEARS`.
- `--out <path>` — output SQLite path. Default `data/seed_2025_26.sqlite`.
- `--cache-dir <path>` — file-cache root. Default `data/cache`.
- `--overrides <path>` — manual rating/contract override TOML. Default
  `data/rating_overrides.toml` (file is optional).
- `--keep-existing` — don't drop+recreate the output DB. By default the
  binary is fully idempotent; existing files are removed first.

## What gets seeded

| Table          | Source                                         |
| -------------- | ---------------------------------------------- |
| `teams` (30)   | static — hardcoded NBA team table              |
| `players`      | Basketball-Reference team rosters (HTML scrape) |
| contracts      | HoopsHype salary table (best-effort, see below) |
| draft prospects| baked-in 2026 mock board (top 60)               |
| ratings        | derived from per-game box stats (BPM-adjacent)  |

The first run takes **~3 minutes** because of the BBRef rate limit
(1 request per 3 seconds × 30 teams + HoopsHype). Every page is cached at
`data/cache/{source}/{key}.{ext}` with a 30-day TTL for HTML and 7 days
for JSON. Subsequent runs hit the cache and finish in under 10 seconds.

## Python `nba_api` (optional)

Stage 2 of the pipeline shells out to Python for `stats.nba.com` advanced
stats (USG%, TS%). Without it, ratings derive purely from BBRef per-game
totals — usable, but coarser.

To enable:

```bash
pip install nba_api
```

If `python3 -c "import nba_api"` fails, the scraper prints a remediation
banner and continues without the augmentation. We never auto-install
Python packages.

## Sports-Reference ToS

Sports-Reference's terms forbid building tools on their data and forbid
training AI models on it. We treat BBRef as a **one-time bootstrap only**:
1 request per 3 seconds (matches their `Crawl-delay`), aggressive 30-day
caching, identifying `User-Agent`. **Do not redistribute** scraped HTML,
do not train models on it. For ongoing data prefer `stats.nba.com` via
`nba_api`.

## Manual overrides

`data/rating_overrides.toml` lets you spot-fix rating curves and surface
contract metadata that the scrapers can't reliably capture:

```toml
[[player]]
name = "Jayson Tatum"
overall = 95
potential = 96
no_trade_clause = true
trade_kicker_pct = 15
```

This is the right home for trade kickers, no-trade clauses, and player
options — they're best-effort to scrape and only need ~50–80 rows per
season (sourced from Hoops Rumors annual articles + Spotrac spot-checks).

## Known gaps

- **HoopsHype contracts**: as of late April 2026 the salary dashboard is
  React-rendered behind a CSS-module class scheme, so the static-HTML
  parser gets 0 rows. The scraper continues without contracts; the seed
  still produces 30 teams + 450-600 players + 60 prospects, which is what
  acceptance requires. Contracts can be filled in via overrides until a
  proper API or DOM-stable mirror lands.
- **Trade kickers, NTCs, options**: not scrape-friendly. Use overrides.
- **Rookie scale & 2026 draft contracts**: M5 work — picks / draft
  contract math lives there, not here.

## Sanity assertions (`src/assertions.rs`)

Every run ends with hardcoded post-scrape checks; any failure exits
non-zero so a broken scrape never produces a quietly-bad seed:

- 30 teams.
- 450..=600 active players.
- 13..=20 players per team.
- ≥60 draft prospects.
- No duplicate player IDs.
- Every player has a non-empty primary position.
- League-wide first-year salary within a loose ±50% band of `30 × cap`
  (warn-only when ≤50 contracts captured — the offline path).
