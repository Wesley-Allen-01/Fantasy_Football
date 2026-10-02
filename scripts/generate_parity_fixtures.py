#!/usr/bin/env python3
"""Regenerate offline golden outputs using the unchanged Python reference.

Python dependencies: requests, requests-mock (the reference's unit-test setup).
Every requests.get is intercepted by requests-mock; unmatched routes fail closed.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import sys

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]


def encode(value):
    return json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n"


def unwrap(value):
    return value[0] if isinstance(value, list) else value


def player_ids(team):
    entries = team.get("roster", {}).get("entries", [])
    if not entries:
        return []
    selected = [entries[0]]
    second = next((e for e in entries[1:] if e["playerPoolEntry"]["player"]["id"] < 0), None)
    if second is None:
        second = next((e for e in entries[1:] if 0 in e["playerPoolEntry"]["player"]["eligibleSlots"]), None)
    if second is not None:
        selected.append(second)
    return [e["playerPoolEntry"]["player"]["id"] for e in selected]


def compact_league(payload):
    """Keep every schedule row/team, but only representative roster entries.

    Player totals retain original values/order including irrelevant-season and
    split-type records so that filtering is exercised. Huge ranking, news and
    breakdown maps are outside the first-slice projection and omitted.
    """
    result = copy.deepcopy(unwrap(payload))
    result.pop("draftDetail", None)
    result["members"] = []
    for matchup in result["schedule"]:
        for side in ["home", "away"]:
            if side in matchup:
                matchup[side] = {k: v for k, v in matchup[side].items() if k in {"teamId", "totalPoints"}}
    for team in result["teams"]:
        selected = player_ids(team)
        team["owners"] = []
        entries = [e for e in team.get("roster", {}).get("entries", []) if e["playerPoolEntry"]["player"]["id"] in selected]
        team["roster"] = {"entries": entries}
        for entry in entries:
            player = entry["playerPoolEntry"]["player"]
            for key in list(player):
                if key not in {"id", "fullName", "eligibleSlots", "proTeamId", "defaultPositionId", "injuryStatus", "injured", "ownership", "stats", "jersey"}:
                    del player[key]
            for stat in player.get("stats", []):
                for key in list(stat):
                    if key not in {"seasonId", "statSplitTypeId", "statSourceId", "scoringPeriodId", "appliedTotal", "appliedAverage"}:
                        del stat[key]
    return [result] if isinstance(payload, list) else result


def oracle(payload, scoreboard, season):
    # Imported only after CLI reference path is configured.
    from espn_api.football import League
    from espn_api.requests.constant import FANTASY_BASE_ENDPOINT
    import requests_mock

    league_id = unwrap(payload)["id"]
    base = FANTASY_BASE_ENDPOINT + "ffl"
    league_url = (f"{base}/leagueHistory/{league_id}?seasonId={season}" if season < 2018
                  else f"{base}/seasons/{season}/segments/0/leagues/{league_id}")
    join = "&" if "?" in league_url else "?"
    # Constructor's auxiliary loads are explicitly empty: this projection uses
    # roster payload identities, not the name map, NFL schedules or draft picks.
    with requests_mock.Mocker(real_http=False) as mock:
        mock.get(league_url + join + "view=mTeam&view=mRoster&view=mMatchup&view=mSettings&view=mStandings", json=payload, complete_qs=True)
        mock.get(league_url + join + "view=mDraftDetail", json={"draftDetail": {"drafted": False}}, complete_qs=True)
        mock.get(f"{base}/seasons/{season}/players?view=players_wl", json=[], complete_qs=True)
        mock.get(f"{base}/seasons/{season}?view=proTeamSchedules_wl", json={"settings": {"proTeams": []}}, complete_qs=True)
        mock.get(league_url + join + "view=mMatchupScore", json=scoreboard, complete_qs=True)
        league = League(league_id, season)
        raw_teams = {t["id"]: t for t in unwrap(payload)["teams"]}
        teams = []
        for team in league.teams:
            selected = player_ids(raw_teams[team.team_id])
            schedule = []
            rows = [m for m in unwrap(payload)["schedule"] if team.team_id in (m.get("home", {}).get("teamId"), m.get("away", {}).get("teamId"))]
            for index, opponent in enumerate(team.schedule):
                bye = opponent.team_id == team.team_id
                schedule.append({"period": rows[index]["matchupPeriodId"], "opponent": None if bye else opponent.team_id,
                                 "score": team.scores[index], "outcome": team.outcomes[index],
                                 "margin": None if bye else team.mov[index]})
            roster = []
            for player in team.roster:
                if player.playerId not in selected:
                    continue
                stats = {str(period): {k: s[k] for k in ["points", "avg_points", "projected_points", "projected_avg_points"] if k in s}
                         for period, s in sorted(player.stats.items())}
                roster.append({"id": player.playerId, "name": player.name, "position": player.position or None,
                               "pro_team": player.proTeam, "eligible_slots": player.eligibleSlots,
                               "total_points": player.total_points, "projected_total_points": player.projected_total_points,
                               "avg_points": player.avg_points, "projected_avg_points": player.projected_avg_points, "stats": stats})
            teams.append({"id": team.team_id, "name": team.team_name, "abbreviation": team.team_abbrev,
                          "wins": team.wins, "losses": team.losses, "ties": team.ties,
                          "points_for": team.points_for, "points_against": team.points_against,
                          "standing": team.standing, "final_standing": team.final_standing,
                          "roster": roster, "schedule": schedule})
        scoreboards = {}
        for period in sorted({1, league.current_week}):
            raw_rows = [r for r in unwrap(scoreboard)["schedule"] if r["matchupPeriodId"] == period]
            scoreboards[str(period)] = [
                {"period": period, "home_team": m._home_team_id if "home" in raw else None,
                 "away_team": m._away_team_id if "away" in raw else None,
                 "home_score": m.home_score if "home" in raw else None,
                 "away_score": m.away_score if "away" in raw else None,
                 "winner": raw["winner"], "matchup_type": m.matchup_type, "is_playoff": m.is_playoff}
                for m, raw in zip(league.scoreboard(period), raw_rows)
            ]
        settings = league.settings
        return {"league_id": league.league_id, "season": league.year, "current_week": league.current_week, "nfl_week": league.nfl_week,
                "current_matchup_period": league.currentMatchupPeriod, "scoring_period": league.scoringPeriodId,
                "first_scoring_period": league.firstScoringPeriod, "final_scoring_period": league.finalScoringPeriod,
                "previous_seasons": league.previousSeasons,
                "settings": {"name": settings.name, "team_count": settings.team_count, "scoring_type": settings.scoring_type,
                             "regular_season_matchup_count": settings.reg_season_count, "playoff_team_count": settings.playoff_team_count,
                             "playoff_seed_tie_rule": settings.playoff_seed_tie_rule, "median_scoring": settings.median_scoring},
                "teams": teams, "standings": [t.team_id for t in league.standings()], "scoreboards": scoreboards}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, default=Path(os.environ.get("ESPN_PYTHON_REFERENCE", ROOT / "reference")))
    parser.add_argument("--check", action="store_true", help="Verify checked-in fixtures without writing")
    args = parser.parse_args()
    sys.path.insert(0, str(args.reference.resolve()))
    source_dir = args.reference / "tests/football/unit/data"
    output_dir = ROOT / "tests/fixtures"
    outputs = {}
    provenance = {}
    for season in [2015, 2018]:
        league_path = source_dir / f"league_{season}_data.json"
        score_path = source_dir / f"league_matchupScore_{season}.json"
        original = json.loads(league_path.read_text())
        scoreboard = json.loads(score_path.read_text())
        compact = compact_league(original)
        expected = oracle(original, scoreboard, season)
        compact_expected = oracle(compact, scoreboard, season)
        if expected != compact_expected:
            raise AssertionError(f"{season}: compact payload changed the Python semantic projection")
        outputs[f"football_{season}_league.json"] = compact
        outputs[f"football_{season}_scoreboard.json"] = scoreboard
        outputs[f"football_{season}_expected.json"] = expected
        for path in [league_path, score_path]:
            provenance[str(path.relative_to(args.reference))] = hashlib.sha256(path.read_bytes()).hexdigest()
    outputs["provenance.json"] = {"generator": "scripts/generate_parity_fixtures.py", "source_sha256": provenance,
                                  "normalization": "Representative roster entries; all teams/schedule; bye self-reference and absent scoreboard sides become null. Actual/projected totals and averages retain Python rounding."}
    for filename, value in outputs.items():
        path = output_dir / filename
        content = encode(value)
        if args.check:
            if not path.exists() or path.read_text() != content:
                raise SystemExit(f"Fixture differs: {path}; regenerate and review")
        else:
            output_dir.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        print(f"{'Verified' if args.check else 'Generated'} {filename}")


if __name__ == "__main__":
    main()
