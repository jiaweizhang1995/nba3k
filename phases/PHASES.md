# nba3k-claude — Phase Tracker

Project status board. Each phase has a dedicated `phases/M{N}-{slug}.md` doc with goals, sub-tasks, agent assignments, and verification artifacts.

| #  | Phase                              | Status      | Started     | Completed   | Bash verification |
|----|------------------------------------|-------------|-------------|-------------|-------------------|
| M1 | [Skeleton + persistence](M1-skeleton.md)         | ✅ Done     | 2026-04-25  | 2026-04-25  | `nba3k new --team BOS --save x.db && nba3k --save x.db status --json` ✅ |
| M2 | [Seed data + sim engine](M2-seed-sim.md)         | ✅ Done     | 2026-04-25  | 2026-04-25  | Full season sim → standings sum = 1230 ✅ (2.8s wall time) |
| M3 | [Trade engine v1 (headline)](M3-trade.md) | ✅ Done | 2026-04-25 | 2026-04-25 | Engine + CBA + negotiation + CLI integration ✅; calibration is polish |
| M4 | [Realism Engine](M4-realism.md)    | ✅ Done     | 2026-04-25  | 2026-04-25  | Luka untradeable ✅, star stat realism (Luka 33/6/15, Jokic 31/12/6) ✅, M4-polish calibration ✅ |
| M5 | [Realism v2 (2K-borrow)](M5-realism-v2.md) | 🔄 Active   | 2026-04-25  | —           | Pending: 21-attribute schema, Role+chemistry, progression, awards, playoffs |
| M6 | Draft + offseason                  | ⏸ Blocked  | —           | —           | Pending: draft → save/load → next season |
| M7 | Polish + AI initiation + integ test| ⏸ Blocked  | —           | —           | Pending: scripted full season, deterministic |

## Working agreements

- **Each phase ends with a Bash-verifiable artifact.** No phase signs off without the assertion command from its doc passing.
- **Per-phase doc is updated continuously** during the phase: sub-task status, decisions made, deviations from plan, blockers surfaced.
- **Phases M2+ use agent teams** (TeamCreate) with non-overlapping crate ownership. Orchestrator (main session) does integration + verification.
- **Memory layer** in `~/.claude/projects/.../memory/` captures durable project context that survives across sessions. Per-phase docs capture in-flight work.

## Agent team conventions

- One team per active phase. Team name format: `nba3k-m{N}`.
- Worker assignments are by **crate path ownership** (e.g., `nba3k-scrape`) so workers don't collide.
- Integration/glue work (CLI wiring, end-to-end verification) is done by orchestrator after workers complete, not by workers themselves.
- Workers communicate progress via `TaskUpdate` on the shared team task list. Orchestrator monitors via `TaskList`.

## Documents

- `RESEARCH.md` — open-items research output (one-shot, refresh as needed)
- `phases/M{N}-*.md` — per-phase plan + log
- `~/.claude/plans/bubbly-roaming-thacker.md` — original approved plan (immutable reference)
