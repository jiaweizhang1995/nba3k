# nba3k — project documentation

Index page. Pick the doc that matches the question:

| You want to know… | Read |
|---|---|
| What is this system? | [`AGENTS.md`](AGENTS.md) (orient + onboard) · [`../README.md`](../README.md) (user-facing CLI reference, in Chinese) |
| How is it organized? | [`ARCHITECTURE.md`](ARCHITECTURE.md) |
| How do I run it? | [`RUNNING.md`](RUNNING.md) · root [`Makefile`](../Makefile) |
| How do I verify changes? | [`VERIFICATION.md`](VERIFICATION.md) |
| Where are we now / what's done? | [`PROGRESS.md`](PROGRESS.md) · [`../phases/PHASES.md`](../phases/PHASES.md) (per-milestone log) |

Conventions:

- Every milestone (`M{N}`) gets a per-phase doc under `phases/M{N}-*.md`. PHASES.md is the status board.
- Schema lives only as `.sql` migrations under `crates/nba3k-store/migrations/`. Numbering doubles as a phase changelog.
- `data/*.toml` is content (archetypes, weights, sim params) — not config. Treat changes as balance work.
- `data/seed_2025_26.sqlite` is the read-only league seed. Every `new` clones it.

If you are an AI agent starting a new session, read `AGENTS.md` first — it is the entry door.
