# QA Report — 2k-tester

## Summary
- **Player identity**: severely broken. Every player in seed is age 25, OVR clustered 72-74. No morale/role/salary surfaced. Doesn't feel like 2K MyGM at all.
- **Roles & morale**: backend wired up but invisible to the user. `player` and `roster` commands hide both fields.
- **Chemistry**: penalties exist but are tiny — a 4-Star stack only drops chemistry from 0.520 to 0.495.
- **Trade evaluator**: untouchables, self-trade, CBA roster bounds work. But: trades can be "accepted" without actually moving the player, free dumps (Brown-for-nothing) are accepted, peer-OVR swaps are flagged "insufficient value".
- **Awards**: MVP/DPOY/Sixth Man populated. ROY, MIP, COY always null (charter mostly anticipates this for ROY). Finals MVP missing entirely from `season-summary`.
- **Progression**: real-life 41yo LeBron and 18yo Cooper Flagg both age 25→26 and BOTH gain +2 OVR after `season-advance`. No old/young divergence.
- **Draft**: board ranking is sane (sorted by potential, ages 18-24). Order is **alphabetical by team abbrev**, not reverse-record — major MyGM violation.

## Findings

### F-01 — Every NBA player is age 25, OVR 72-74 in seed — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k.db roster BOS
BOS roster (16 players):
ID     NAME                          POS  AGE  OVR  POT
688883990  Payton Pritchard              PG    25   74   80
1931031263  Jayson Tatum                  PF    25   73   79
1918566053  Jaylen Brown                  SF    25   72   78
...
$ ./target/release/nba3k --save /tmp/qa-2k.db roster LAL
1074844123  LeBron James                  SF    25   72   78
4234747350  Luka Dončić                   PG    25   74   80
$ ./target/release/nba3k --save /tmp/qa-2k.db roster DAL
459652346  Cooper Flagg                  SF    25   72   78
108617221  Kyrie Irving                  SG    25   73   79
```
**Expected**: NBA 2K MyGM has wildly varied ages (18yo rookies → 41yo LeBron) and OVRs (60s for end-of-bench → 96+ for Jokić/SGA). Stars are visibly stars.
**Actual**: Every player aged exactly 25. Every OVR between 72 and 74. LeBron and Cooper Flagg are indistinguishable from a deep bench role player. The seed is structurally non-2K.
**Fix idea**: Either a calibration step that imports real ages and OVR distributions from a source table, or a procedural generator that respects realistic age curves and OVR variance (σ≈8) by player role.

### F-02 — `player` command reveals no role, morale, salary, experience, or contract — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k.db player "Jayson Tatum"
Jayson Tatum (PF) — BOS
age 25 | OVR 73 | POT 79

$ ./target/release/nba3k --save /tmp/qa-2k.db player "Jayson Tatum" --json
{ "age":25, "name":"Jayson Tatum", "overall":73, "potential":79,
  "no_trade_clause": false, "trade_kicker_pct": null, "ratings":{...} }
```
**Expected**: 2K MyGM player card shows role tag, morale bar, salary, contract years, experience years, free-agent year, NTC flag. JSON has `no_trade_clause` and `trade_kicker_pct` but neither is shown in text output.
**Actual**: Only 3 numbers (age/OVR/POT) plus position. No morale, no role, no salary, no contract.
**Fix idea**: Add role + morale to the text card (they exist — `roster-set-role` mutates them). Even if salary/contract are unmodeled, role and morale are core MyGM info and they're already in the DB.

### F-03 — Setting morale via `roster-set-role` works, but setting same role twice is a no-op silently — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-2.db roster-set-role "Jayson Tatum" star
Jayson Tatum: role -> Star (morale 0.80)
$ ./target/release/nba3k --save /tmp/qa-2k-2.db roster-set-role "Jayson Tatum" bench
Jayson Tatum: role -> BenchWarmer (morale 0.00)        # Δ = -0.80
$ ./target/release/nba3k --save /tmp/qa-2k-2.db roster-set-role "Jayson Tatum" star
Jayson Tatum: role -> Star (morale 0.40)               # Δ = +0.40 only
$ ./target/release/nba3k --save /tmp/qa-2k-2.db roster-set-role "Jayson Tatum" star
Jayson Tatum: role -> Star (morale 0.40)               # no change but no message
```
**Expected (2K MyGM)**: A demoted star drops morale ~0.4 and may demand a trade. Promoting to star fully restores morale (or close to). Idempotent role-set should print "no change".
**Actual**: Star→Bench drops by 0.80 (twice 2K's typical). Bench→Star only adds 0.40 (so re-promoting doesn't recover). Setting same role twice silently keeps morale at 0.40.
**Fix idea**: Symmetrize the morale delta between role transitions. Print `(no change)` when the role is unchanged.

### F-04 — No trade demand surface for unhappy stars — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-3.db roster-set-role "Jayson Tatum" bench
Jayson Tatum: role -> BenchWarmer (morale 0.30)
$ ./target/release/nba3k --save /tmp/qa-2k-3.db status
save:     /tmp/qa-2k-3.db
season:   2026 (PreSeason)
day:      0
team:     BOS (id=2)
...
$ ./target/release/nba3k --save /tmp/qa-2k-3.db trade list
  ID  STATUS    ROUND  TEAMS
```
**Expected**: A demoted Star with 0.30 morale should generate a trade demand event (in MyGM the player walks into your office). Status or a dedicated `messages`/`inbox` should surface it.
**Actual**: No surface. The user has no way to know Tatum is unhappy.
**Fix idea**: Add `messages` or `inbox` subcommand. At minimum, surface a count in `status` ("3 player concerns").

### F-05 — Star-stack chemistry penalty is too small to feel — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-4.db chemistry BOS
chemistry BOS: 0.520
  baseline               +0.700
  role distribution      -0.100
  positional balance     -0.080
  scheme fit             +0.000
  morale                 +0.000

$ for p in "Jayson Tatum" "Jaylen Brown" "Derrick White" "Payton Pritchard"; do
    ./target/release/nba3k --save /tmp/qa-2k-4.db roster-set-role "$p" star
  done
$ ./target/release/nba3k --save /tmp/qa-2k-4.db chemistry BOS
chemistry BOS: 0.495
  baseline               +0.700
  role distribution      -0.140
  positional balance     -0.080
  morale                 +0.015
  scheme fit             +0.000
```
**Expected**: Stacking 4 stars on one team should cause a clear chemistry tension in MyGM (think "too many alphas" — visible drop, often visible in player chemistry tooltips).
**Actual**: Only -0.025 net. Role distribution penalty barely moves (-0.04 for adding 4 stars). Net morale even went *up* because their personal morale jumped to 0.80 each.
**Fix idea**: Steepen role-distribution penalty above 2 stars. Add a separate "ball share / usage conflict" reason for star-stacks.

### F-06 — Scheme fit is hard-zero — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k.db chemistry BOS
  scheme fit             +0.000
$ ./target/release/nba3k --save /tmp/qa-2k-4.db chemistry BOS
  scheme fit             +0.000
$ ./target/release/nba3k --save /tmp/qa-2k-4.db chemistry BOS --json
... "delta":0.0,"label":"scheme fit" ...
```
**Expected**: Scheme fit should vary by team — drive-and-kick teams want shooters around their slasher, defensive teams want long wings. This is a marquee MyGM/MyLeague feature.
**Actual**: Always exactly 0.000 across multiple teams and roster configurations.
**Fix idea**: Either implement scheme-fit scoring against a team scheme record, or remove the line until it carries weight.

### F-07 — "Free dump" trade accepted, but player did not actually move — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-7.db trade propose \
    --from BOS --to LAL --send "Jaylen Brown"
trade #2 — verdict: accept | status: accepted | round: 1 | teams: BOS/LAL
$ ./target/release/nba3k --save /tmp/qa-2k-7.db player "Jaylen Brown"
Jaylen Brown (SF) — BOS
age 25 | OVR 72 | POT 78
```
**Expected**: 2K MyGM: a one-sided dump must be balanced by another asset (or AI counters with picks). And if a trade is "accepted," the player should immediately move to the new team.
**Actual**: AI accepted the dump (free Jaylen Brown for nothing). Worse: even though `trade list` shows "accepted", Jaylen Brown is still on BOS. Trade execution does not actually mutate the roster.
**Fix idea**: Two bugs in one: (a) `accept` for an empty receive side is wrong — should at minimum require salary parity / asset parity / a pick the AI demands; (b) accepted trades must atomically move players into the receiving team.

### F-08 — Peer-OVR equal-asset trade rejected as `insufficientvalue` — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-7.db trade propose \
    --from BOS --to DAL --send "Jordan Walsh" --receive "Brandon Williams"
trade #4 — verdict: reject(insufficientvalue) | status: rejected | round: 1 | teams: BOS/DAL
```
Both players are 25yo. Walsh is OVR 73 / POT 79; Williams is OVR 74 / POT 80. Difference is one point — well within MyGM "fair" range.
**Expected**: A 1-OVR delta peer swap should accept or counter. MyGM trade finder rates this 50/50.
**Actual**: Rejected as insufficient value. The evaluator over-discounts incoming asset slightly above the sent asset.
**Fix idea**: Inspect the value formula. The 1-pt OVR gap is being treated as a much bigger gap. Possibly an asymmetric "loss aversion" coefficient on the AI side that's too harsh. Add a counter-offer path instead of outright reject when within ~3 OVR.

### F-09 — Awards: ROY, MIP, COY are always null — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-5.db awards
Awards (season 2026):
  MVP        Devin Booker
  DPOY       Bam Adebayo
  ROY        —
  Sixth Man  Sharife Cooper  (TW)
  MIP        —
  COY        —
```
**Expected**: ROY null is acceptable (charter notes M5 doesn't track is_rookie). But MIP and COY should be implementable from existing data — MIP from progression deltas (we have them: `Δsum=1160` was logged), COY from team record vs. preseason expectation or pure best record.
**Actual**: Only 3 of 6 awards populate. Sixth Man went to a player with `(TW)` two-way contract, which in real MyGM is ineligible (two-way players can't win 6MOY).
**Fix idea**: (a) MIP = top OVR Δ from the progression pass we already log. (b) COY = team that exceeded its preseason projection by most wins (or just best regular-season record if that's not modeled). (c) Filter `(TW)` players from Sixth Man eligibility.

### F-10 — Finals MVP missing from `season-summary` — severity: medium

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-6.db playoffs sim
... Champion: BOS
$ ./target/release/nba3k --save /tmp/qa-2k-6.db season-summary
Season 2026 summary:
  champion : BOS
  MVP        Devin Booker
  DPOY       Bam Adebayo
  ROY        —
  Sixth Man  Sharife Cooper  (TW)
  MIP        —
$ ./target/release/nba3k --save /tmp/qa-2k-6.db season-summary --json
{ "awards": {...}, "champion": "BOS", "season": 2026 }
```
**Expected**: Charter says "champion + finals MVP + awards bundle". Finals MVP is one of the most iconic MyGM moments.
**Actual**: Finals MVP not in text and not in JSON. Also COY is omitted from text (it's listed in `awards` but dropped from `season-summary`).
**Fix idea**: Track playoff scoring leader on champion's roster as Finals MVP; add field to summary text + JSON.

### F-11 — Progression bumps every player +OVR regardless of age — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-5.db player "Cooper Flagg"
Cooper Flagg (SF) — DAL    age 25 | OVR 72 | POT 78
$ ./target/release/nba3k --save /tmp/qa-2k-5.db player "LeBron James"
LeBron James (SF) — LAL    age 25 | OVR 72 | POT 78
$ ./target/release/nba3k --save /tmp/qa-2k-5.db season-advance
advanced to season 2027 — progressed 530 players (Δsum=1160), 30 drafted
$ ./target/release/nba3k --save /tmp/qa-2k-5.db player "Cooper Flagg"
Cooper Flagg (SF) — DAL    age 26 | OVR 74 | POT 78    # +2 OVR
$ ./target/release/nba3k --save /tmp/qa-2k-5.db player "LeBron James"
LeBron James (SF) — LAL    age 26 | OVR 74 | POT 78    # +2 OVR
$ ./target/release/nba3k --save /tmp/qa-2k-5.db roster BOS
... every BOS player went 25→26, OVR 73→75 or 74→77, all +2/+3 ...
```
**Expected (2K MyGM)**: 18yo with high potential gains 2-5 OVR. 35yo regresses 1-3 OVR. Stars in their prime (~26-29) gain 0-1 or stay flat.
**Actual**: Everyone gains roughly the same. Of course, this is partly a downstream of F-01 (everyone is 25 to start). But the progression engine itself doesn't appear to read age tiers — even if ages were varied, this probably gives the same lockstep bump.
**Fix idea**: Tie progression to age curve: <22 = aggressive growth toward potential, 22-29 = mild fluctuation, 30+ = decay starting at -0.3/yr accelerating after 33. Verify by spot-checking the ΔOVR per age bucket after seed is fixed (F-01).

### F-12 — Draft order is alphabetical, not reverse-record — severity: high

**What I did**:
```
$ ./target/release/nba3k --save /tmp/qa-2k-5.db draft order
Draft order:
  Pick  1: ATL
  Pick  2: BOS
  Pick  3: BRK
  Pick  4: CHO
  Pick  5: CHI
  Pick  6: CLE
  ...
  Pick 30: WAS
```
**Expected (2K MyGM)**: Worst regular-season team gets pick 1 (with lottery odds). Order is by inverse standings. The fact that BOS won the 2026 championship and got pick 2 is impossible in real MyGM.
**Actual**: Picks 1..30 are in alphabetical order of the team abbreviation. The completed standings are ignored.
**Fix idea**: Sort `draft order` by `(W ASC, PCT ASC)` from the previous regular season. Optionally model a 14-team lottery.

(Bonus observation, not formally a finding) Trade-evaluator CBA check is solid: 12-player floor, 18-player ceiling correctly rejected. Untouchable list (LeBron, Luka) works. Self-trade rejected with a clean message. These pieces are all 2K-faithful.

