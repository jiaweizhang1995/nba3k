# M17 — GM tools: offers + extensions + notes

## Scope

3 parallel workers.

## Pre-locked CLI

```
Command::Offers { limit, json }                  → cmd_offers     (worker-a)
Command::Extend { player, salary_m, years }      → cmd_extend     (worker-b)
Command::Notes(NotesAction::Add|Remove|List)     → cmd_notes_*    (worker-c)
```

## Worker A — Incoming AI offers

**Owned crates**:
- `crates/nba3k-store/src/store.rs` — read trade chains where `initiator != user_team` and `status = open`.
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_offers`.

**Goal**: `nba3k offers` lists incoming trade offers from AI teams targeting your players. Auto-generated daily during `sim_n_days` for AI teams that have an interest signal (M16-B logic).

1. **Daily auto-generation** in `sim_n_days`: with low probability (e.g. 1% / day per AI team, scaled by GM aggression), an AI team picks one user-team player they want, packages a counter, and submits.
   - Use `evaluate_mod::evaluate` from user's perspective to predict if user would accept; only submit offers that have a >40% chance of acceptance (don't spam garbage).
   - Persist via `Store::insert_trade_chain`.
2. **`cmd_offers`**:
   - Read open chains where `initiator != user_team` AND user_team is involved.
   - Render top-N:
     ```
     Incoming offers:
       ID  FROM  WANTS              SENDS                          VERDICT (your side)
        7  LAL   Jaylen Brown       Reaves, Russell, 2027 1st     counter (insufficient value)
       12  GSW   Derrick White      Hield, Kuminga                accept
     ```
   - JSON variant.
3. User can use existing `trade respond <id> accept|reject|counter` to act on these (already wired).

Tests in `crates/nba3k-cli/tests/offers_smoke.rs`:
- Auto-gen pass produces ≥ 1 offer over 30-day sim.
- `cmd_offers` text includes user team name on receiving side.

## Worker B — Contract extensions

**Owned crates**:
- `crates/nba3k-models/src/contract_extension.rs` (new) — pure function for accept/reject decision.
- `crates/nba3k-cli/src/commands.rs` — body of `cmd_extend`.
- Tests in `crates/nba3k-models/tests/contract_extension.rs`.

**Goal**: user proposes extension; system evaluates fairness vs market rate (from `contract_gen::generate_contract`) + adjusts for player happiness.

1. **`accept_extension(player, offered_salary, offered_years, season) -> ExtensionDecision`**:
   - Compare offered_salary vs `contract_gen::generate_contract(player, season).years[0].salary`.
   - Accept if `offered_salary >= 0.95 * market` AND `offered_years` is reasonable (3-5 typical).
   - Counter if `offered_salary >= 0.85 * market`. Return `Counter { request_salary, request_years }`.
   - Reject below 0.85 with reason.
   - Boost acceptance for happy players (morale > 0.7) — discount factor 5%.

2. **`cmd_extend`**:
   - Resolve player on user team. Refuse if not on user team.
   - Build offer.
   - Call `accept_extension`.
   - On Accept: append `ContractYear` rows to `player.contract.years` for the new term, persist via `upsert_player`. News kind=`extension`.
   - On Counter: print counter terms; user can re-run with the new numbers.
   - On Reject: print reason.

Tests:
- Above-market offer → Accept.
- Below-market → Reject with reason.
- Near-market → Counter with reasonable bump.

## Worker C — Notes / favorites

**Owned crates**:
- `crates/nba3k-store/migrations/V012__notes.sql`.
- `crates/nba3k-store/src/store.rs` — `insert_note(player_id, text)`, `delete_note(player_id)`, `list_notes() -> Vec<NoteRow>`.
- `crates/nba3k-cli/src/commands.rs` — bodies of `cmd_notes_add`, `cmd_notes_remove`, `cmd_notes_list`.

Migration:
```sql
CREATE TABLE notes (
    player_id INTEGER PRIMARY KEY REFERENCES players(id),
    text TEXT,
    created_at TEXT NOT NULL
);
```

**Goal**: lightweight player favorites tracker. Notes show up in a prominent place (extend `cmd_messages` to add a "Notes:" section).

1. **`cmd_notes_add`**: resolve player, upsert note row.
2. **`cmd_notes_remove`**: delete note for player.
3. **`cmd_notes_list`**: walk notes, join with players → render with current OVR/team/contract.
4. Surface in `cmd_messages`: append a "Notes (N tracked players):" section listing names.

Tests in `crates/nba3k-store/tests/notes.rs`:
- Add → list returns the row.
- Remove → list empty.
- Updating existing player's note overwrites text.

## Acceptance

```bash
rm -f /tmp/m17.db
./target/release/nba3k --save /tmp/m17.db new --team BOS

# Worker A: offers
./target/release/nba3k --save /tmp/m17.db sim-day 30
./target/release/nba3k --save /tmp/m17.db offers

# Worker B: extension
./target/release/nba3k --save /tmp/m17.db extend "Jayson Tatum" --salary-m 50 --years 4

# Worker C: notes
./target/release/nba3k --save /tmp/m17.db notes add "Cooper Flagg" --text "watch as draft target"
./target/release/nba3k --save /tmp/m17.db notes list
./target/release/nba3k --save /tmp/m17.db messages   # Notes section
```

## Working agreements

- DO NOT touch `crates/nba3k-cli/src/cli.rs`.
- `cargo test --workspace` green at every commit boundary.
- TaskUpdate completed + send `team-lead` "done — N files, M tests" + go idle.
