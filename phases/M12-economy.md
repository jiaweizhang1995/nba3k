# M12 — League economy

## Scope

Complete the salary economy + endgame so the league feels lived-in:
- Every roster player has a real contract from seed (not just FA-signed).
- Retired players don't disappear — show up in a Hall of Fame view.
- AI teams compete in free agency during off-season (don't just stand still).

3 parallel workers, crate-ownership split.

## Pre-locked CLI (DO NOT EDIT)

```
Command::Hof { limit, json }   → cmd_hof   (worker-b)
```

(`cmd_hof` body is a stub that bails until worker-b fills it.)

Workers A and C don't add new CLI; they extend existing flows (`new` / `season-advance`).

## Worker A — Contract backfill at scrape

**Owned crates**: `crates/nba3k-scrape/src/seed.rs` (and a small helper in `nba3k-models::contract_gen` if needed).

**Goal**: every active player ends up with a non-NULL contract after `cargo run -p nba3k-scrape`. The cap command should show realistic team payrolls ($120-180M typical).

1. Read the existing pipeline in `crates/nba3k-scrape/src/seed.rs`. After players are written to the DB, walk every roster player (`team_id IS NOT NULL`).
2. For each, call `nba3k_models::contract_gen::generate_contract(&player, season)`.
3. Persist via `Store::upsert_player` with the new `player.contract = Some(generated)`.
4. **Add minor variance**: don't every-OVR-90 player getting the same flat contract. Use a deterministic per-player hash to perturb salary by ±10% so e.g. Tatum vs Brown vs Curry don't all read $55M flat.
5. Verify: post-scrape, run `cap BOS` — payroll should land $130-160M for a typical team. League payroll should sum to ~$5-6B (30 teams × ~$170M).

Tests: not strict — extend existing scraper assertions in `crates/nba3k-scrape/src/assertions.rs`:
- After write, every `team_id IS NOT NULL` row has `contract_json IS NOT NULL`.
- Sum of league salaries ∈ `[$3B, $7B]` (loose band; salary cap × 30 ≈ $4.6B with mild over-cap teams).

Run `cargo run -p nba3k-scrape --release -- --season 2025-26 --out data/seed_2025_26.sqlite` to verify post-fix output.

## Worker B — Hall of Fame

**Owned crates**: `crates/nba3k-store/src/store.rs` (already has `list_retired_players` from M11), body of `cmd_hof` in `crates/nba3k-cli/src/commands.rs`.

**Goal**: `nba3k --save x.db hof` shows retired players ranked by career production (career PTS as primary key; tiebreaker on career WS-equivalent like `pts + 2.5*ast + 1.2*reb`).

1. Walk `Store::list_retired_players()`.
2. For each, call `Store::read_career_stats(player_id)` (worker-b's M10 API).
3. Compute career totals: total PTS, RPG, APG, GP, championships (0 for now).
4. Sort by career PTS desc, take top `limit`.
5. Print:
```
Hall of Fame (top 10):
RANK  NAME              POS  YRS  GP    PTS    RPG  APG
   1  LeBron James      SF    25  1421  41523  7.4  7.8
   2  Stephen Curry     PG    18  1156  28934  4.5  6.6
   ...
```
6. JSON variant emits structured array.

Tests in `crates/nba3k-store/tests/hof.rs` (or a new `crates/nba3k-cli/tests/...` smoke test):
- Empty save (no retired players) prints "No retired players yet."
- After `retire`-ing a player, `hof` includes them.

## Worker C — AI free-agent market

**Owned crates**: `crates/nba3k-cli/src/commands.rs` (extend `cmd_season_advance`), no new files needed.

**Goal**: during `season-advance`, AI teams sign top free agents. Order: highest cap-room team picks first, signs the best-available FA whose contract demand fits, repeat until top-N FAs are signed or all teams full.

1. After draft auto-sim in `cmd_season_advance`, run a free-agency pass:
   - Sort AI teams (skip user team) by `cap_room = cap - team_salary` descending.
   - For each team in order, while the team has < 16 players AND there are unsigned FAs:
     - Pick highest-OVR free agent.
     - Generate contract via `contract_gen::generate_contract`.
     - If team_salary + first_year_salary <= cap × 1.30 (allowing some over-cap), sign them.
     - Else skip (try next FA).
2. Print summary in `season-advance` output: `... 30 drafted, 26 retired, 12 FAs signed`.
3. Skip the user's team — leave their FA decisions to `nba3k fa sign` manual.

Tests: extend `crates/nba3k-cli/tests/integration_season1.rs` to assert that after `season-advance`, FA pool count went down (some FAs got signed).

## Acceptance

```bash
# Worker A: contracts populate at seed
cargo run -p nba3k-scrape --release -- --season 2025-26 --out data/seed_2025_26.sqlite
rm -f /tmp/m12.db
./target/release/nba3k --save /tmp/m12.db new --team BOS
./target/release/nba3k --save /tmp/m12.db cap BOS  # payroll > $100M

# Worker B: HOF
./target/release/nba3k --save /tmp/m12.db retire "LeBron James"
./target/release/nba3k --save /tmp/m12.db hof --limit 5

# Worker C: AI FA market
./target/release/nba3k --save /tmp/m12.db sim-to playoffs
./target/release/nba3k --save /tmp/m12.db season-advance  # "X FAs signed"
./target/release/nba3k --save /tmp/m12.db fa list | head  # fewer entries
```

## Working agreements

- Each worker keeps `cargo test --workspace` green at every commit boundary.
- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
