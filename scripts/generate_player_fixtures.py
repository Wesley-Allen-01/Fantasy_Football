#!/usr/bin/env python3
"""Offline Python oracle for football free agents, cards and lookup requests.

Dependencies: requests and requests-mock. Every request is closed-mocked.
"""
from __future__ import annotations
import argparse
from collections import Counter
import copy
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import sys
import time
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
NOW = 1728230400000


def encode(value):
    return json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + "\n"


class FrozenDatetime(datetime):
    @classmethod
    def now(cls, tz=None):
        return cls.fromtimestamp(NOW / 1000.0, tz)


def stat(season, period, source, points, team):
    return {"seasonId": season, "scoringPeriodId": period, "statSplitTypeId": 1,
            "statSourceId": source, "proTeamId": team, "appliedTotal": points, "appliedAverage": points / 2,
            "stats": {"43": 1 if source == 0 else 2, "44": 1},
            "appliedStats": {"43": 6 if source == 0 else 12, "44": 2}}


def wrapper(player_id, name, slots, position, team, actual, projected, injured=False):
    stats = [stat(2024, 0, 0, 200.345, team), stat(2024, 0, 1, 225.675, team)]
    if actual is not None:
        stats.append(stat(2024, 7, 0, actual, 11 if team == 16 and position == 3 else team))
    if projected is not None:
        stats.append(stat(2024, 7, 1, projected, team))
    ignored = stat(2024, 7, 0, 9999, team)
    ignored["statSplitTypeId"] = 2
    stats += [ignored, stat(2023, 1, 0, 8888, team)]
    player = {"id": player_id, "fullName": name, "eligibleSlots": slots, "defaultPositionId": position,
              "proTeamId": team, "jersey": "18", "positionalRanking": 5,
              "injuryStatus": "QUESTIONABLE" if injured else "ACTIVE", "injured": injured,
              "ownership": {"percentOwned": 97.555, "percentStarted": 25.125}, "stats": stats}
    return {"id": player_id, "onTeamId": 0, "acquisitionType": "FREEAGENT", "player": player,
            "status": "FREEAGENT", "transactions": []}


def schedule():
    date = NOW - 3 * 60 * 60 * 1000
    return {"settings": {"proTeams": [
        {"id": 16, "proGamesByScoringPeriod": {"7": [{"homeProTeamId": 16, "awayProTeamId": 12, "date": date}],
             "8": [{"homeProTeamId": 16, "awayProTeamId": 11, "date": date + 604800000}]}},
        {"id": 11, "proGamesByScoringPeriod": {"7": [{"homeProTeamId": 11, "awayProTeamId": 12, "date": date - 1},
             {"homeProTeamId": 11, "awayProTeamId": 16, "date": date + 5000}]}},
        {"id": 12, "proGamesByScoringPeriod": {"7": [{"homeProTeamId": 11, "awayProTeamId": 12, "date": date - 1}]}},
        {"id": 23, "proGamesByScoringPeriod": {"1": [{"homeProTeamId": 23, "awayProTeamId": 17, "date": 1567976400000}],
             "2": [{"homeProTeamId": 12, "awayProTeamId": 23, "date": 1568581200000},
                   {"homeProTeamId": 23, "awayProTeamId": 16, "date": 1568581300000}]}},
        {"id": 0, "proGamesByScoringPeriod": {}},
    ]}}


def ratings():
    return {"positionAgainstOpponent": {"positionalRatings": {
        "1": {"ratingsByOpponent": {"12": {"rank": 0}}},
        "3": {"ratingsByOpponent": {"12": {"rank": 4}}},
        "16": {"ratingsByOpponent": {}},
    }}}


def synthetic():
    players = [wrapper(7002, "Alex Doe", [3, 4, 5, 7, 20, 21, 23], 3, 16, 18.567, 20.255, True),
               wrapper(7001, "Alex Doe", [0, 7, 20, 21], 1, 16, None, 19.345),
               wrapper(-16011, "Colts D/ST", [16, 20, 21], 16, 11, 10.125, None)]
    return {"name": "synthetic_2024_free_agents", "evidence": "synthetic modern response; not a captured current ESPN league",
            "context": {"season": 2024, "scoring_period": 7, "now_unix_ms": NOW, "offset": 0, "limit": 3},
            "players": {"players": players}, "pro_schedule": schedule(), "positional_ratings": ratings()}


def aux_maps(value, period):
    pro = {}
    all_pro = {}
    for team in value.get("settings", {}).get("proTeams", []):
        games = team.get("proGamesByScoringPeriod", {})
        all_pro[team["id"]] = games
        selected = games.get(str(period), [])
        if team["id"] and selected:
            game = selected[0]
            opponent = game["homeProTeamId"] if team["id"] == game["awayProTeamId"] else game["awayProTeamId"]
            pro[team["id"]] = (opponent, game["date"])
    return pro, all_pro


def ranking_map(value):
    return {p: {o: r["rank"] for o, r in entry["ratingsByOpponent"].items()}
            for p, entry in value.get("positionAgainstOpponent", {}).get("positionalRatings", {}).items()}


def nullable(value):
    return None if value == [] or value == "" else value


def player_projection(player, aliases):
    def breakdown(value):
        return {str(k): v for k, v in value.items() if k not in aliases}
    stats = {}
    for period, values in sorted(player.stats.items()):
        lines = {}
        for name, prefix in [("actual", ""), ("projected", "projected_")]:
            point_key = prefix + "points"
            if point_key in values:
                lines[name] = {"points": values[point_key], "average_points": values[prefix + "avg_points"],
                               "breakdown": breakdown(values[prefix + "breakdown"]),
                               "points_breakdown": breakdown(values[prefix + "points_breakdown"])}
        stats[str(period)] = lines
    jersey = nullable(player.jersey)
    return {"id": player.playerId, "name": player.name, "position": nullable(player.position),
            "eligible_slots": player.eligibleSlots, "pro_team": player.proTeam,
            "positional_rank": nullable(player.posRank), "jersey": None if jersey is None else str(jersey),
            "acquisition_type": nullable(player.acquisitionType), "on_team_id": nullable(player.onTeamId),
            "lineup_slot": nullable(player.lineupSlot), "injury_status": nullable(player.injuryStatus),
            "injured": player.injured, "percent_owned": player.percent_owned, "percent_started": player.percent_started,
            "active_status": player.active_status, "total_points": player.total_points,
            "projected_total_points": player.projected_total_points, "avg_points": player.avg_points,
            "projected_avg_points": player.projected_avg_points, "stats": stats}


def free_agent_projection(player, raw, rankings, aliases):
    from espn_api.football.constant import PRO_TEAM_MAP
    nested = raw.get("player", raw.get("playerPoolEntry", {}).get("player"))
    opponent = next((id for id, label in PRO_TEAM_MAP.items() if label == player.pro_opponent), None)
    has_rank = opponent is not None and str(opponent) in rankings.get(str(nested["defaultPositionId"]), {})
    return {"player": player_projection(player, aliases), "slot_position": player.slot_position,
            "pro_team": player.proTeam, "pro_opponent": None if player.pro_opponent == "None" else player.pro_opponent,
            "pro_pos_rank": player.pro_pos_rank if has_rank else None,
            "game_date_unix_ms": round(player.game_date.timestamp() * 1000) if hasattr(player, "game_date") else None,
            "game_played": player.game_played, "on_bye_week": player.on_bye_week,
            "points": player.points, "projected_points": player.projected_points}


def oracle_free_agents(value, aliases):
    from espn_api.football import BoxPlayer
    import requests_mock
    context = value["context"]
    pro, _ = aux_maps(value["pro_schedule"], context["scoring_period"])
    rankings = ranking_map(value["positional_ratings"])
    with requests_mock.Mocker(real_http=False), patch("espn_api.football.box_player.datetime", FrozenDatetime):
        players = [BoxPlayer(raw, pro, rankings, context["scoring_period"], context["season"]) for raw in value["players"]["players"]]
    return [free_agent_projection(p, raw, rankings, aliases) for p, raw in zip(players, value["players"]["players"])]


def oracle_cards(value, aliases):
    from espn_api.football import Player
    import requests_mock
    _, schedules = aux_maps(value["pro_schedule"], 0)
    result = []
    with requests_mock.Mocker(real_http=False):
        for raw in value["cards"]["players"]:
            player = Player(raw, value["season"], schedules)
            result.append({"player": player_projection(player, aliases),
                           "schedule": {str(period): {"opponent": game["team"], "date_unix_ms": round(game["date"].timestamp() * 1000)} for period, game in player.schedule.items()},
                           "raw_transactions": raw.get("transactions", raw.get("player", {}).get("transactions", []))})
    return result


def compact_cards(original):
    value = copy.deepcopy(original)
    for wrapper in value["players"]:
        player = wrapper["player"]
        for key in ["rankings", "draftRanksByRankType", "seasonOutlook", "lastNewsDate", "lastVideoDate", "firstName", "lastName"]:
            player.pop(key, None)
    return value


def traces(mock):
    return [{"method": r.method, "path": urlsplit(r.url).path, "query": parse_qs(urlsplit(r.url).query),
             "fantasy_filter": json.loads(r.headers["x-fantasy-filter"]) if "x-fantasy-filter" in r.headers else None}
            for r in mock.request_history]


def request_oracle(value):
    from espn_api.football import League
    from espn_api.requests.constant import FANTASY_BASE_ENDPOINT
    import requests_mock
    base = FANTASY_BASE_ENDPOINT + "ffl/seasons/2024"
    url = base + "/segments/0/leagues/123"
    cases = []
    for name, kwargs in [("free_agents_default", {}), ("free_agents_filtered", {"week": 7, "size": 3, "position": "WR", "position_id": 23}),
                         ("free_agents_future", {"week": 99}), ("free_agents_explicit_qb_zero", {"position_id": 0}),
                         ("free_agents_qb_label", {"position": "QB"}), ("free_agents_empty", {})]:
        league = League(123, 2024, fetch_league=False)
        league.current_week = 7
        week = kwargs.get("week", 7)
        response = {"players": []} if name.endswith("empty") else value["players"]
        with requests_mock.Mocker(real_http=False) as mock, patch("espn_api.football.box_player.datetime", FrozenDatetime):
            mock.get(url + f"?view=kona_player_info&scoringPeriodId={week}", json=response, complete_qs=True)
            mock.get(base + "?view=proTeamSchedules_wl", json=value["pro_schedule"], complete_qs=True)
            mock.get(url + f"?view=mPositionalRatings&scoringPeriodId={week}", json=value["positional_ratings"], complete_qs=True)
            result = league.free_agents(**kwargs)
            cases.append({"name": name, "arguments": kwargs, "requests": traces(mock), "result_ids": [p.playerId for p in result]})
    directory = [{"id": 7002, "fullName": "Alex Doe"}, {"id": 7001, "fullName": "Alex Doe"},
                 {"id": -16011, "fullName": "Colts D/ST"}, {"id": 7002, "fullName": "Alex Doe"}]
    league = League(123, 2024, fetch_league=False)
    league.finalScoringPeriod = 17
    with requests_mock.Mocker(real_http=False) as mock:
        mock.get(base + "/players?view=players_wl", json=directory, complete_qs=True)
        league._fetch_players()
        cases.append({"name": "player_directory", "requests": traces(mock), "python_first_name_id": league.player_map["Alex Doe"]})
    for name, kwargs, response in [("player_card_id", {"playerId": 7002}, {"players": [value["players"]["players"][0]]}),
                                  ("player_card_many", {"playerId": [7001, -16011, 7002]}, value["players"]),
                                  ("player_card_name", {"name": "Alex Doe"}, {"players": [value["players"]["players"][0]]}),
                                  ("player_card_unknown_name", {"name": "Unknown"}, None),
                                  ("player_card_case_sensitive_name", {"name": "alex doe"}, None),
                                  ("player_card_empty", {"playerId": 7002}, {"players": []})]:
        with requests_mock.Mocker(real_http=False) as mock:
            if response is not None:
                mock.get(url + "?view=kona_playercard", json=response, complete_qs=True)
                mock.get(base + "?view=proTeamSchedules_wl", json=value["pro_schedule"], complete_qs=True)
            result = league.player_info(**kwargs)
            players = result if isinstance(result, list) else [result] if result is not None else []
            cases.append({"name": name, "arguments": kwargs, "requests": traces(mock), "result_ids": [p.playerId for p in players]})
    with requests_mock.Mocker(real_http=False) as mock:
        mock.get(url + "?view=kona_player_info&scoringPeriodId=7", [
            {"json": {"players": [{"id": 7002}, {"id": 7001}]}},
            {"json": {"players": [{"id": 7001}, {"id": -16011}]}},
            {"json": {"players": []}},
        ], complete_qs=True)
        ids = league.espn_request.get_player_pool_ids(7, page_size=2)
        cases.append({"name": "shared_player_pool_pagination", "requests": traces(mock), "result_ids": ids})
    old = League(123, 2018, fetch_league=False)
    with requests_mock.Mocker(real_http=False) as mock:
        try:
            old.free_agents()
        except Exception as error:
            cases.append({"name": "free_agents_historical_gate", "season": 2018, "requests": traces(mock), "error": str(error)})
        else:
            raise AssertionError("Python historical free-agent gate unexpectedly accepted 2018")
    return {"cases": cases,
            "deviations": {"explicit_qb_zero": "Python numeric position_id=0 is ignored; Rust explicit SlotId(0) filters QB. Python position='QB' already includes zero.",
                           "duplicate_names": "Python chooses first directory name ID 7002; Rust returns both distinct IDs 7002/7001 in response order.",
                           "pagination": "Football free_agents has no offset argument; Rust adds explicit pages. Captured player-pool pagination is shared request evidence, not an exact free-agent API match.",
                           "cards": "Python player_info sends its ID collection as one request; Rust deduplicates and batches 40 IDs. Raw transactions are retained by Rust, not modeled as typed transaction history."}}, {"season": 2024, "input": directory,
             "expected": [{"id": 7002, "name": "Alex Doe"}, {"id": 7001, "name": "Alex Doe"}, {"id": -16011, "name": "Colts D/ST"}],
             "queries": {"Alex Doe": [7002, 7001], "Colts D/ST": [-16011], "alex doe": [], "Unknown": []}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, default=Path(os.environ.get("ESPN_PYTHON_REFERENCE", ROOT / "reference")))
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    os.environ["TZ"] = "UTC"
    time.tzset()
    sys.path.insert(0, str(args.reference.resolve()))
    from espn_api.football.constant import PLAYER_STATS_MAP
    aliases = sorted(label for label, count in Counter(PLAYER_STATS_MAP.values()).items() if count > 1)
    value = synthetic()
    empty = copy.deepcopy(value)
    empty["name"] = "synthetic_2024_empty_free_agents"
    empty["players"] = {"players": []}
    future = copy.deepcopy(value)
    future["name"] = "synthetic_2024_future_week"
    future["context"]["scoring_period"] = 99
    free_agents = [value, empty, future]
    source = args.reference / "tests/football/unit/data/league_2019_playerCard.json"
    original = json.loads(source.read_text())
    cards = {"evidence": "captured 2019 playerCard fixture with synthetic supplemental NFL schedule; no modern live capture",
             "season": 2019, "cards": compact_cards(original), "pro_schedule": schedule()}
    full = copy.deepcopy(cards)
    full["cards"] = original
    expected = oracle_cards(full, aliases)
    assert expected == oracle_cards(cards, aliases), "Card compaction changed Python semantic projection"
    requests, directory = request_oracle(value)
    outputs = {"free_agents_input.json": {"cases": free_agents},
               "free_agents_expected.json": {"cases": [{"name": case["name"], "players": oracle_free_agents(case, aliases)} for case in free_agents]},
               "historical_2019_cards_input.json": cards, "historical_2019_cards_expected.json": expected,
               "synthetic_2024_cards_input.json": {"season": 2024, "cards": value["players"], "pro_schedule": value["pro_schedule"], "evidence": "synthetic modern cards"},
               "synthetic_2024_cards_expected.json": oracle_cards({"season": 2024, "cards": value["players"], "pro_schedule": value["pro_schedule"]}, aliases),
               "python_requests.json": requests, "directory.json": directory,
               "provenance.json": {"generator": "scripts/generate_player_fixtures.py", "clock_unix_ms": NOW, "timezone": "UTC",
                   "source": "tests/football/unit/data/league_2019_playerCard.json", "source_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
                   "excluded_duplicate_stat_labels": aliases,
                   "normalization": "Python missing recursive fields [] / empty labels=>null; jersey=>text; ranks distinguish absent versus present zero; duplicate stat-label aliases excluded only from label comparisons; numeric IDs remain authoritative. Card transactions retained raw, not a typed history parity claim.",
                   "evidence_limits": "2019 card source; synthetic schedules and modern FA/cards; no live current/private ESPN response."}}
    target = ROOT / "tests/fixtures/players"
    for name, output in outputs.items():
        content = encode(output)
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
