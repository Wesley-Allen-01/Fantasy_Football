#!/usr/bin/env python3
"""Generate deterministic offline Python football weekly-box-score goldens.

Dependencies: requests, requests-mock. No real ESPN requests are permitted.
"""
from __future__ import annotations
import argparse
import copy
from collections import Counter
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import sys
import time
from types import SimpleNamespace
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
NOW = 1728230400000


def encode(value):
    return json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n"


class FrozenDatetime(datetime):
    now_ms = NOW

    @classmethod
    def now(cls, tz=None):
        return cls.fromtimestamp(cls.now_ms / 1000.0, tz)


def entry(player_id, slot=4, current_team=16, actual=None, projected=12.345, actual_team=None, position=3, season=2024):
    stats = []
    for source, points in [(0, actual), (1, projected)]:
        if points is not None:
            stats.append({"seasonId": season, "statSplitTypeId": 1, "scoringPeriodId": 7,
                          "statSourceId": source, "proTeamId": actual_team if source == 0 and actual_team is not None else current_team,
                          "appliedTotal": points, "appliedAverage": points,
                          "stats": {"43": 1 if source == 0 else 2, "44": 1 if source == 0 else 2},
                          "appliedStats": {"43": 6 if source == 0 else 12, "44": 2 if source == 0 else 4}})
    player = {"id": player_id, "fullName": f"Synthetic Player {player_id}", "proTeamId": current_team,
              "defaultPositionId": position, "eligibleSlots": [4, 20, 21, 23], "injuryStatus": "ACTIVE",
              "injured": False, "ownership": {"percentOwned": 80, "percentStarted": 50}, "stats": stats}
    return {"playerId": player_id, "lineupSlotId": slot, "acquisitionType": "DRAFT",
            "playerPoolEntry": {"id": player_id, "onTeamId": 1, "player": player}}


def side(team_id, players, score=25.555, live=None, projected=None):
    result = {"teamId": team_id, "totalPoints": score, "rosterForCurrentScoringPeriod": {"entries": players}}
    if live is not None:
        result["totalPointsLive"] = live
    if projected is not None:
        result["totalProjectedPointsLive"] = projected
    return result


def aux_schedule():
    date = NOW - 3 * 60 * 60 * 1000
    # Multiple games: the first game is authoritative for the requested week.
    return {"settings": {"proTeams": [
        {"id": 11, "proGamesByScoringPeriod": {"7": [{"homeProTeamId": 11, "awayProTeamId": 12, "date": date}, {"homeProTeamId": 11, "awayProTeamId": 16, "date": date + 100000}], "8": [{"homeProTeamId": 11, "awayProTeamId": 12, "date": date}]}},
        {"id": 12, "proGamesByScoringPeriod": {"7": [{"homeProTeamId": 11, "awayProTeamId": 12, "date": date}], "8": [{"homeProTeamId": 11, "awayProTeamId": 12, "date": date}]}},
        {"id": 16, "proGamesByScoringPeriod": {"7": [{"homeProTeamId": 16, "awayProTeamId": 11, "date": date + 1000}]}},
        {"id": 0, "proGamesByScoringPeriod": {}},
    ]}}


def aux_ratings():
    return {"positionAgainstOpponent": {"positionalRatings": {
        "3": {"ratingsByOpponent": {"12": {"rank": 4}, "11": {"rank": 0}}},
        "2": {"ratingsByOpponent": {}},
    }}}


def case(name, matchup, history=None, now=NOW, season=2024, scoring_period=7, matchup_period=4):
    return {"name": name, "evidence": "synthetic modern model characterization",
            "context": {"season": season, "scoring_period": scoring_period, "matchup_period": matchup_period,
                        "now_unix_ms": now, "initial_history": history or {}},
            "box_scores": {"schedule": [matchup]}, "pro_schedule": aux_schedule(), "positional_ratings": aux_ratings()}


def synthetic_cases():
    starters = [entry(1001, actual=18.567, projected=20.255, actual_team=11),
                entry(1002, slot=20, actual=4, projected=99),
                entry(1003, slot=21, current_team=17, actual=None, projected=88),
                entry(1004, slot=23, current_team=12, actual=2, projected=4.444)]
    fallback = {"id": 101, "matchupPeriodId": 4, "playoffTierType": "WINNERS_BRACKET",
                "home": side(1, starters, score=100.123, live=95.987), "winner": "UNDECIDED"}
    live = {"id": 102, "matchupPeriodId": 4, "home": side(1, starters, score=200, live=100.125, projected=110.555),
            "away": side(2, [entry(2001, current_team=16, actual=5.555, projected=9.999, position=2)], score=35.555, projected=999), "winner": "HOME"}
    only_projection = entry(1001, current_team=34, actual=None, projected=13.456)
    stale = entry(3001, current_team=16, actual=50, projected=None, actual_team=11, season=2023)
    stale["playerPoolEntry"]["player"]["stats"].append({"seasonId": 2024, "scoringPeriodId": 7, "statSourceId": 1,
        "proTeamId": 16, "appliedTotal": 17.777, "stats": {"43": 2}, "appliedStats": {"43": 12}})
    split = entry(3002, current_team=16, actual=7, projected=8, actual_team=0)
    split["playerPoolEntry"]["player"]["stats"][0]["statSplitTypeId"] = 2
    return [case("starter_fallback_at_three_hours", fallback),
            case("starter_fallback_after_three_hours", copy.deepcopy(fallback), now=NOW + 1),
            case("live_totals_override_and_missing_rank", live),
            case("projected_only_uses_history_without_updating", {"id": 103, "matchupPeriodId": 4,
                 "away": side(2, [only_projection]), "winner": "UNDECIDED"}, {"1001": 11}),
            case("actual_team_overrides_history", {"id": 104, "matchupPeriodId": 4,
                 "home": side(1, [entry(1001, actual=3, actual_team=12)]), "winner": "HOME"}, {"1001": 11}),
            case("actual_team_search_and_stat_filters", {"id": 105, "matchupPeriodId": 4,
                 "home": side(1, [stale, split]), "winner": "TIE"})]


def decode_aux(case_input):
    week = case_input["context"]["scoring_period"]
    schedule = {}
    for team in case_input["pro_schedule"].get("settings", {}).get("proTeams", []):
        games = team.get("proGamesByScoringPeriod", {}).get(str(week), [])
        if team["id"] != 0 and games:
            game = games[0]
            opponent = game["homeProTeamId"] if team["id"] == game["awayProTeamId"] else game["awayProTeamId"]
            schedule[team["id"]] = (opponent, game["date"])
    ratings = {position: {opponent: data["rank"] for opponent, data in rating["ratingsByOpponent"].items()}
               for position, rating in case_input["positional_ratings"].get("positionAgainstOpponent", {}).get("positionalRatings", {}).items()}
    return schedule, ratings


def project(case_input, boxes, history, aliases):
    from espn_api.football.constant import PRO_TEAM_MAP
    _, rankings = decode_aux(case_input)
    def breakdown(data):
        return {str(k): v for k, v in data.items() if k not in aliases}
    def player_projection(player, raw):
        nested = raw["playerPoolEntry"]["player"] if "playerPoolEntry" in raw else raw["player"]
        # Python uses sentinel rank 0 for absence. Preserve a present rank 0.
        opponent_id = next((k for k, v in PRO_TEAM_MAP.items() if v == player.pro_opponent), None)
        ranking = rankings.get(str(nested["defaultPositionId"]), {})
        has_rank = opponent_id is not None and str(opponent_id) in ranking
        return {"id": player.playerId, "name": player.name, "slot_position": player.slot_position,
                "pro_team": player.proTeam, "pro_opponent": None if player.pro_opponent == "None" else player.pro_opponent,
                "pro_pos_rank": player.pro_pos_rank if has_rank else None,
                "game_date_unix_ms": round(player.game_date.timestamp() * 1000) if hasattr(player, "game_date") else None,
                "game_played": player.game_played, "on_bye_week": player.on_bye_week,
                "points": player.points, "projected_points": player.projected_points,
                "breakdown": breakdown(player.breakdown), "points_breakdown": breakdown(player.points_breakdown),
                "projected_breakdown": breakdown(player.projected_breakdown),
                "projected_points_breakdown": breakdown(player.projected_points_breakdown)}
    matchups = []
    for box, raw in zip(boxes, case_input["box_scores"]["schedule"]):
        sides = {}
        for side_name in ["home", "away"]:
            if side_name not in raw:
                sides[side_name] = None
            else:
                lineup = getattr(box, side_name + "_lineup")
                entries = raw[side_name]["rosterForCurrentScoringPeriod"]["entries"]
                sides[side_name] = {"team_id": raw[side_name]["teamId"], "score": getattr(box, side_name + "_score"),
                                    "projected": getattr(box, side_name + "_projected"),
                                    "lineup": [player_projection(p, r) for p, r in zip(lineup, entries)]}
        matchups.append({"id": raw.get("id"), **sides, "matchup_type": box.matchup_type, "is_playoff": box.is_playoff})
    return {"season": case_input["context"]["season"], "scoring_period": case_input["context"]["scoring_period"],
            "matchup_period": case_input["context"]["matchup_period"], "matchups": matchups,
            "history": {str(k): v for k, v in sorted(history.items())}}


def oracle(case_input, aliases):
    from espn_api.football.box_score import BoxScore
    import requests_mock
    context = case_input["context"]
    cache = {int(k): v for k, v in context["initial_history"].items()}
    schedule, ratings = decode_aux(case_input)
    FrozenDatetime.now_ms = context["now_unix_ms"]
    with requests_mock.Mocker(real_http=False), patch("espn_api.football.box_player.datetime", FrozenDatetime):
        boxes = [BoxScore(row, schedule, ratings, context["scoring_period"], context["season"], cache)
                 for row in case_input["box_scores"]["schedule"]]
    return project(case_input, boxes, cache, aliases)


def compact_historical(original, aliases):
    from espn_api.football.constant import PLAYER_STATS_MAP
    # One complete historical matchup: all starters/bench retained, projections
    # stay equivalent. No historical NFL schedule/ranking capture was supplied.
    row = copy.deepcopy(original["schedule"][0])
    for side_name in ["home", "away"]:
        if side_name not in row:
            continue
        original_side = row[side_name]
        row[side_name] = {k: original_side[k] for k in ["teamId", "totalPoints", "totalPointsLive", "totalProjectedPointsLive"] if k in original_side}
        entries = original_side["rosterForCurrentScoringPeriod"]["entries"]
        row[side_name]["rosterForCurrentScoringPeriod"] = {"entries": entries}
        for e in entries:
            p = e["playerPoolEntry"]["player"]
            for key in list(p):
                if key not in {"id", "fullName", "eligibleSlots", "proTeamId", "defaultPositionId", "injuryStatus", "injured", "ownership", "stats", "jersey"}:
                    del p[key]
            for stat in p.get("stats", []):
                for key in list(stat):
                    if key not in {"seasonId", "statSplitTypeId", "statSourceId", "scoringPeriodId", "proTeamId", "appliedTotal", "appliedAverage", "stats", "appliedStats"}:
                        del stat[key]
                for field in ["stats", "appliedStats"]:
                    if field in stat:
                        stat[field] = {k: v for k, v in stat[field].items() if PLAYER_STATS_MAP.get(int(k), k) not in aliases}
    value = case("historical_2018_first_matchup", row, season=2018, scoring_period=original["scoringPeriodId"], matchup_period=row["matchupPeriodId"])
    value["evidence"] = "selected original 2018 payload; direct BoxScore model only, not a supported modern League.box_scores response"
    value["pro_schedule"] = {"settings": {"proTeams": []}}
    value["positional_ratings"] = {}
    return value


def characterize_requests(synthetic, aliases):
    from espn_api.football import League
    from espn_api.requests.constant import FANTASY_BASE_ENDPOINT
    import requests_mock
    case_input = copy.deepcopy(synthetic)
    FrozenDatetime.now_ms = case_input["context"]["now_unix_ms"]
    league = League(123, 2024, fetch_league=False)
    league.current_week = 8
    league.currentMatchupPeriod = 4
    league.settings = SimpleNamespace(matchup_periods={"3": [5, 6], "4": [7, 8]})
    base = FANTASY_BASE_ENDPOINT + "ffl/seasons/2024"
    league_url = base + "/segments/0/leagues/123"
    results = []
    for requested_week in [7, None, 99]:
        week = 7 if requested_week == 7 else 8
        current = copy.deepcopy(case_input)
        current["context"]["scoring_period"] = week
        cache = {}
        with requests_mock.Mocker(real_http=False) as mock, patch("espn_api.football.box_player.datetime", FrozenDatetime):
            mock.get(league_url + f"?view=mMatchupScore&view=mScoreboard&scoringPeriodId={week}", json=current["box_scores"], complete_qs=True)
            mock.get(base + "?view=proTeamSchedules_wl", json=current["pro_schedule"], complete_qs=True)
            mock.get(league_url + f"?view=mPositionalRatings&scoringPeriodId={week}", json=current["positional_ratings"], complete_qs=True)
            boxes = league.box_scores(requested_week, cache)
            requests = [{"method": r.method, "path": urlsplit(r.url).path,
                         "query": parse_qs(urlsplit(r.url).query),
                         "fantasy_filter": json.loads(r.headers["x-fantasy-filter"]) if "x-fantasy-filter" in r.headers else None}
                        for r in mock.request_history]
            results.append({"requested_week": requested_week, "resolved_scoring_period": week, "resolved_matchup_period": 4,
                            "requests": requests, "output": project(current, boxes, cache, aliases)})
    old = League(123, 2018, fetch_league=False)
    with requests_mock.Mocker(real_http=False) as mock:
        try:
            old.box_scores(7)
        except Exception as error:
            gate = {"season": 2018, "error": str(error), "request_count": len(mock.request_history)}
        else:
            raise AssertionError("Python historical box score gate unexpectedly accepted 2018")
    return {"cases": results, "historical_gate": gate}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, default=Path(os.environ.get("ESPN_PYTHON_REFERENCE", ROOT / "reference")))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    # The Python package uses naive local datetimes. Make this deterministic,
    # then normalize exported dates to epoch milliseconds.
    os.environ["TZ"] = "UTC"
    time.tzset()
    sys.path.insert(0, str(args.reference.resolve()))
    from espn_api.football.constant import PLAYER_STATS_MAP
    aliases = sorted(label for label, count in Counter(PLAYER_STATS_MAP.values()).items() if count > 1)
    cases = synthetic_cases()
    outputs = {"synthetic_2024_input.json": {"cases": cases},
               "synthetic_2024_expected.json": {"cases": [{"name": c["name"], "output": oracle(c, aliases)} for c in cases]}}
    source = args.reference / "tests/football/unit/data/league_boxscore_2018.json"
    original = json.loads(source.read_text())
    compact = compact_historical(original, aliases)
    full = copy.deepcopy(compact)
    full["box_scores"] = {"schedule": [original["schedule"][0]]}
    expected = oracle(full, aliases)
    assert expected == oracle(compact, aliases), "Compaction changed historical Python projection"
    outputs["historical_2018_input.json"] = compact
    outputs["historical_2018_expected.json"] = expected
    outputs["python_requests.json"] = characterize_requests(cases[0], aliases)
    outputs["provenance.json"] = {"generator": "scripts/generate_box_score_fixtures.py", "clock_timezone": "UTC",
        "source": "tests/football/unit/data/league_boxscore_2018.json", "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
        "historical_selection": "First matchup, all 30 lineup entries; direct model characterization; no 2018 schedule/rank capture; modern cases synthetic",
        "normalization": "Absent sides/opponents/dates/ranks=>null; missing rank0 versus explicit rank0 distinguished by raw mapping. Duplicate stat-label aliases excluded from label comparison; numeric IDs remain authoritative in Rust.",
        "excluded_duplicate_stat_labels": aliases}
    target = ROOT / "tests/fixtures/weekly"
    for name, value in outputs.items():
        content = encode(value)
        path = target / name
        if args.check:
            if not path.exists() or path.read_text() != content:
                raise SystemExit(f"Fixture differs: {path}; regenerate and review")
        else:
            target.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        print(f"{'Verified' if args.check else 'Generated'} {name}")


if __name__ == "__main__":
    main()
