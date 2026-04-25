# nba3k-claude — Open-Item Research (2026-04-25)

## Executive Summary
Riskiest unknowns: (1) Sports-Reference now explicitly bans scraping for AI training and any "tools/sites built on scraped data" — even personal-use scraping is policy-prohibited, though technically possible at low rates; (2) Spotrac/HoopsHype/Stathead all carry similar ToS prohibitions and tight rate limits (10 req/min on SR-family sites); (3) `stats.nba.com` remains undocumented and unstable — Python `nba_api` (swar/nba_api) is the only well-maintained client and the Rust `nba_api` crate (parasm) is essentially unmaintained (0 stars, no releases); (4) trade-kicker mechanics around the apron are non-trivial — kicker counts as incoming-only, prorated by remaining guaranteed years, and must be modeled per cap year; (5) NBA scheduling is NP-hard with proprietary constraints — no public production-grade open-source generator exists; we will need a heuristic. Everything else (CBA numbers, BBGM rating heuristics, BPM regression skeleton) is well-documented and green-lit.

---

## 1. Legality of scraping Basketball-Reference / Sports Reference (2026)

**robots.txt** (`https://www.basketball-reference.com/robots.txt`):
- `Crawl-delay: 3` for `User-agent: *`
- Disallows `/play-index/*.cgi?*`, `/leagues/*/gamelog/`, `/players/*/splits/`, lineup pages, `/req/`, `/short/`, `/nocdn/`
- Full-block list includes `AhrefsBot`, `GPTBot`, `SlySearch`, `GroundControl`, `Carmine`, `Skynet`, `The-Matrix`, `HAL9000`
- Twitterbot allowed everywhere

**Bot/rate policy** (`https://www.sports-reference.com/bot-traffic.html`, `/429.html`):
- Hard rate limit: **20 requests/minute on Basketball-Reference**, **10 req/min on FBref/Stathead**. Violation = 1-hour IP jail.
- Recommended crawl delay = 3 sec/request.

**Data Use** (`https://www.sports-reference.com/data_use.html`, `/termsofuse.html`):
- Explicitly prohibits: (a) using SR data to train/fine-tune/prompt AI models; (b) building "websites or tools" based on scraped SR data without permission; (c) creating competing databases.
- Custom data downloads start at $5,000 minimum.
- Stathead ToS: bans "automated means... scripts, bots, scrapers, data miners" without written permission.

**Verdict:** Per-second, single-IP scraping for a personal CLI is **technically tolerated by the rate-limiter at ≤1 req/3s** (matches the published `Crawl-delay`), but is **policy-prohibited by ToS** for any "tool built on scraped data" — which a GM-mode CLI literally is. Community thresholds: 1 req/3s and ≤20/min are the bright lines; staying under both is the only documented "tolerated" rate. The legal exposure for personal, non-commercial, non-redistributed use is low in practice but contractually non-zero.

**Decision for build:** Use Basketball-Reference as a *one-time bootstrap* source (rosters, historical box scores) at 1 req/3s with a polite `User-Agent` identifying the project and contact email. Cache aggressively to disk (sqlite). Never re-scrape what we already have. Do not redistribute scraped HTML or train any model on it. Add a `--respect-robots` flag default-on. For ongoing/recurring data, prefer `stats.nba.com` (item 2).

---

## 2. stats.nba.com endpoint stability in 2026

- **Accessibility:** Still accessible and undocumented. `swar/nba_api` last release **v1.11.4 on 2026-02-20** — actively maintained (https://github.com/swar/nba_api).
- **Required headers** (well-known consensus from `nba_api` and community issues):
  - `User-Agent: Mozilla/5.0 ...` (any modern desktop UA)
  - `Referer: https://stats.nba.com`
  - `Origin: https://www.nba.com`
  - `x-nba-stats-origin: stats`
  - `x-nba-stats-token: true`
  - `Accept-Language: en-US,en;q=0.9`
  - `Connection: keep-alive`
  - Missing any of these usually returns 403 or empty payload.
- **Stability caveat:** README states "NBA.com does not provide information regarding new, changed, or removed endpoints." Endpoints have historically broken without notice; community reports gate the recovery.
- **Rust-native option:** `nba_api` on crates.io (parasm/nba_api) — **0 stars, 0 releases, page errors loading activity** → effectively dead. `statbook` crate covers NFL only. No viable Rust-native NBA client.

**Decision for build:** Do **not** port endpoints to Rust by hand. Shell out from a thin `scrape` binary to Python `nba_api` via a small subprocess wrapper (json over stdout). Keep the scrape boundary isolated so we can swap in a native Rust HTTP client later if `nba_api` decays. Cache every response to sqlite keyed by `(endpoint, params, season)`. Treat stats.nba.com as best-effort augmentation; Basketball-Reference is the durable bootstrap.

---

## 3. Spotrac / HoopsHype / contract data access

- **Spotrac:** No public API. ToS prohibits automated scraping. Page structure changes occasionally but selectors for `/nba/contracts/` and `/nba/cap/` tables have been stable for ~2 years per community scrapers. Robots.txt unchecked here but ToS is the binding constraint.
- **HoopsHype** (`https://hoopshype.com/salaries/`): Public, JS-light, server-rendered tables. Easy to scrape with `reqwest` + `scraper` crate. Dashboard relaunched Nov 2025 — "Like Basketball-Reference, but for money" (https://www.hoopshype.com/story/sports/nba/2025/11/25/nba-salaries-dashboard-like-basketball-reference-but-for-money/86936637007/). No published API. ToS ambiguous; site has historically been the most scraper-tolerated of the contract sources.
- **Basketball-Insiders:** Has salary pages but inconsistent updates and no API.
- **No-trade clauses, trade kickers, player options:** `hoopsrumors.com` (`https://www.hoopsrumors.com/2024/08/nba-players-with-trade-kickers-in-2024-25.html`) maintains canonical annual lists. These are narrative articles, not structured tables — best ingested as a hand-curated CSV updated yearly. Spotrac contract pages also list NTC/kicker flags per contract.

**Decision for build:** **HoopsHype is the v1 primary source** for player salaries and team cap totals. Scrape `/salaries/players/` and per-team pages at 1 req/3s. Maintain `data/contracts/2025-26-overrides.csv` for trade kickers, NTCs, player/team options, and ETOs — sourced manually from Hoops Rumors annual articles + Spotrac spot-checks. The override CSV beats flaky scraping for these fields and only needs ~50–80 rows per season.

---

## 4. Post-2023 CBA exact thresholds for the 2025-26 league year

Sources: NBA.com (`https://www.nba.com/news/nba-salary-cap-set-2025-26-season`), Hoops Rumors (`https://www.hoopsrumors.com/2025/06/salary-cap-tax-line-set-for-2025-26-nba-season.html`, `/2025/06/values-of-2025-26-mid-level-bi-annual-exceptions.html`, `/2025/08/cash-sent-received-in-nba-trades-for-2025-26.html`), Sports Business Classroom (`https://sportsbusinessclassroom.com/nba-2025-26-apron-tracker/`, `/nba-available-cash-in-trade-2025-26/`).

| Item | 2025-26 amount |
|---|---|
| Salary cap | **$154,647,000** |
| Luxury tax line | **$187,895,000** |
| First apron | **$195,945,000** |
| Second apron | **$207,824,000** |
| Non-taxpayer MLE | **$14,104,000** |
| Taxpayer MLE | **$5,685,000** |
| Room MLE | **$8,781,000** |
| Bi-Annual Exception (BAE) | **$5,134,000** (max 2-yr deal value $10,524,700) |
| Minimum team salary (90% of cap) | **$139,182,000** |
| Max cash sent or received in trades | **$7,964,000** (separate caps; teams above 2nd apron cannot send cash; sending cash imposes a 2nd-apron hard cap) |
| Rookie scale, #1 pick, year-1 (120%) | **~$13.8M** for the 2025 draft class' 2025-26 season (https://sports.yahoo.com/article/revealed-2025-nba-draft-pick-232055016.html); 2026 draft class' #1 projection is ~$12.3M year-1 per SalarySwish — note SalarySwish/Sportico use different "120% standard" conventions, so always store both 100% scale and 120% effective. |

**Decision for build:** Hard-code the 2025-26 numbers as constants in `src/cba/league_year.rs`, keyed by season string. Add a `LeagueYear` struct with all 11 fields above. Source each constant with an inline URL comment so future updates are auditable. Build the rookie scale as a 30-row table (picks 1–30, year 1–4, 100% and 120%) imported from a CSV.

---

## 5. NBA schedule generation algorithm

- **Real NBA constraints** (https://www.sportico.com/leagues/basketball/2022/nba-schedule-2023-rest-travel-1234691876/, league rules): 82 games/team = 1230 total; each team plays divisional opponents 4×, same-conference non-div 3–4×, opposite-conference 2×; max 1 stretch of 4-in-5 nights (recently reduced); back-to-back caps; arena-conflict avoidance; nationally televised game obligations; "series" model (same matchup played twice consecutively in same arena) increasingly common.
- **Academic/open-source:**
  - `jackconnolly21/nba-scheduler` (Python, hill-climbing + simulated annealing on an existing schedule, reduces b2b by ~7/team). **Does not generate from scratch** — only optimizes a seed.
  - CMU OR project paper (`https://www.math.cmu.edu/~af1p/Teaching/OR2/Projects/P49/21-393ProjectPaper_Group1.pdf`) and Bao thesis (Ohio Link) formulate it as binary integer programming / time-relaxed round-robin.
  - No open-source project produces a from-scratch realistic 1230-game NBA schedule end-to-end; the league's actual generator is proprietary (AWS-assisted optimization).
- **Simplest approximation that is "non-laughable":**
  1. Build the matchup matrix from CBA rules (4× div, 4× ½ of intra-conf non-div, 3× other ½, 2× inter-conf) = exactly 1230 game-pairs.
  2. Distribute over a ~170-day window using a greedy round-robin, scheduling each team into open dates with hard constraints: ≤1 game/day, ≥1 day rest after most games, no more than 1 b2b per week, no 4-in-5.
  3. Run a fixup pass: random swaps that reduce a cost function (b2b count + travel-mile estimate via team city lat/lon).
  4. Skip TV scheduling, arena conflicts, NBA Cup tournament dates for v1.

**Decision for build:** Implement a 3-stage generator: **(a) matchup-count solver** (deterministic, CBA-rule driven), **(b) greedy date assignment with hard rest constraints**, **(c) simulated-annealing fixup** minimizing weighted cost (b2b + 4-in-5 violations + travel miles). Target: 1230 games, ≤14 b2b/team, 0 4-in-5 violations. Ship behind `--schedule-quality fast|good` flag. Defer NBA Cup, arena availability, national TV slots to post-v1.

---

## 6. Stats → ratings published methodology

- **BasketballGM** (https://nicidob.github.io/automatic_bbgm/, https://basketball-gm.com/manual/): Player has 15 sub-ratings (height, strength, speed, jumping, endurance, shooting [3 categories], dribbling, passing, rebounding, defense, post-skills). Auto-roster generator uses a regularized linear regression / neural net trained on simulated BBGM seasons mapping `box_stats → ratings`. Source code is open at `dumbmatter/gm-games`.
- **ZenGM family:** Same architecture as BBGM; ratings drive sim, sim drives stats — the inverse mapping is what `nicidob` published.
- **Box Plus/Minus (BPM 2.0)** (`https://www.basketball-reference.com/about/bpm2.html`): Linear regression from box-score stats to RAPM. Coefficients are public. Output is points/100 possessions vs. league average — converting to a 0-99 overall is just a normalization.
- **Sane formula skeleton** (recommended for v1):
  ```
  // 1. Compute BPM from per-100 box stats (use published BBRef coefficients)
  // 2. Split into OBPM and DBPM
  // 3. Map to 0-99 sub-ratings via percentile within position group:
  //    shooting   = f(TS%, 3P%, FT%, 3PAr)
  //    playmaking = f(AST%, AST/TO, USG)
  //    rebounding = f(ORB%, DRB%)
  //    defense    = f(DBPM, STL%, BLK%, opponent FG% on)
  //    finishing  = f(eFG% near rim, FTr)
  //    iq         = f(TOV% inverse, AST/TO, foul rate inverse)
  // 4. Age curve: peak 26-28, decline after 30 (BBGM uses similar)
  // 5. Position weights: PG weights playmaking heavier, C weights rebounding/finishing
  // 6. Overall = weighted avg of sub-ratings, then percentile-normalize to 0-99 across the league
  ```
- Academic mapping work exists (Nature Sci. Reports 2024 hierarchical evaluation paper) but is overkill for v1.

**Decision for build:** Implement BPM 2.0 as the spine (well-documented, deterministic). Derive 6 sub-ratings from BPM components + raw rates, then percentile-normalize to 0-99 against the current league for that season. Apply position-weighted overall. Apply a simple age curve (linear to age 28, -1.5/yr after 30). Tune by spot-checking 20 known players (top stars should land 90+, role players 70-80, fringe 50-60). No ML model in v1.

---

## 7. Trade kicker mechanics — pre vs post matching

- Kicker is up to **15% of the player's remaining base salary** (https://www.basketball-reference.com/contracts/glossary.html, https://www.hoopsrumors.com/2018/12/hoops-rumors-glossary-trade-kickers.html).
- **For salary matching: kicker applies AFTER, but only on the receiving side.**
  - Trading team: matches against the *original* salary (no kicker).
  - Acquiring team: must match against the *new* salary (original + prorated kicker).
  - This asymmetry is exactly why kickers can break trades — the receiving team's incoming number balloons.
- **Proration:** Total kicker is prorated over the remaining guaranteed years (excluding unexercised options) of the contract, then added to each year's cap hit on the new team. Trading mid-year (after July 1 vs before) changes how many seasons it's spread across.
- **Waiver:** Player **may** waive the kicker (entirely or partially) at any time before the trade is finalized — never an obligation. Famous recent example: Anthony Davis waived his kicker for the Lakers→Mavs trade. Once exercised, a kicker is paid by the **trading team** in cash but counted as cap salary by the **acquiring team**.
- Player/team options do not contribute to kicker base unless already exercised; ETOs do count.
- CBA reference: Article VII, Section 3 (Trade Bonuses) of the 2023 CBA. Authoritative reporter coverage: Yossi Gozlan, Larry Coon's CBA FAQ (`http://cbafaq.com/salarycap.htm`), Sports Business Classroom (`https://sportsbusinessclassroom.com/understanding-trade-matching-in-the-new-collective-bargaining-agreement/`).

**Decision for build:** Model kicker as a `TradeKicker { pct: f32, waivable: bool }` field on `Contract`. Trade evaluation function accepts an optional `KickerWaiver` decision per kicker-eligible player. Salary-match math: for each side independently, compute incoming salary using *that side's* counted figure (sender uses pre-kicker, receiver uses post-kicker prorated over remaining guaranteed years). Cap hit on receiver applies starting in the cap year of the trade through end of guaranteed term. Surface in CLI: `nba3k trade simulate --waive-kicker P12345` flag.

---

*Inline URLs used throughout; no separate Sources section needed.*
