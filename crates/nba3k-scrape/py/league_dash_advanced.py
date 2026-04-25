"""Fetch league-wide advanced player stats from stats.nba.com via nba_api.

Argv: season string e.g. "2025-26".
Stdout: JSON array of {name, usage, ts_pct}.
Stderr: traceback on failure (non-zero exit).
"""

import json
import sys


def main() -> int:
    if len(sys.argv) < 2:
        print("usage: league_dash_advanced.py <season>", file=sys.stderr)
        return 2
    season = sys.argv[1]

    try:
        from nba_api.stats.endpoints import leaguedashplayerstats
    except Exception as e:  # noqa: BLE001
        print(f"failed to import nba_api: {e}", file=sys.stderr)
        return 3

    try:
        ep = leaguedashplayerstats.LeagueDashPlayerStats(
            season=season, measure_type_detailed_defense="Advanced", per_mode_detailed="PerGame"
        )
        data = ep.get_normalized_dict()
    except Exception as e:  # noqa: BLE001
        print(f"nba_api request failed: {e}", file=sys.stderr)
        return 4

    rows = data.get("LeagueDashPlayerStats", [])
    out = []
    for r in rows:
        name = r.get("PLAYER_NAME", "")
        usage = r.get("USG_PCT") or 0.0
        ts = r.get("TS_PCT") or 0.0
        out.append({"name": name, "usage": float(usage), "ts_pct": float(ts)})

    json.dump(out, sys.stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
