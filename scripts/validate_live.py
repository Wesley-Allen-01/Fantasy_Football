#!/usr/bin/env python3
"""Capture actual Rust HTTP reads, then verify Rust/Python offline on those bytes.

Dependencies: requests; verify also uses requests-mock and the frozen reference.
Capture folders must be outside the repository and are never uploaded by this tool.
"""
from __future__ import annotations

import argparse
import base64
import copy
from datetime import datetime
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
from unittest.mock import patch
from urllib.parse import parse_qs, urlsplit

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = "https://lm-api-reads.fantasy.espn.com"
ROLES = ("league", "weekly", "weekly_schedule", "weekly_ratings", "free_agents",
         "free_agents_schedule", "free_agents_ratings", "cards", "cards_schedule")
VIEWS = (("mTeam", "mRoster", "mMatchup", "mSettings", "mStandings"),
         ("mMatchupScore", "mScoreboard"), ("proTeamSchedules_wl",), ("mPositionalRatings",),
         ("kona_player_info",), ("proTeamSchedules_wl",), ("mPositionalRatings",),
         ("kona_playercard",), ("proTeamSchedules_wl",))
PLAYER_KEYS = ("id", "name", "position", "eligible_slots", "pro_team", "total_points",
               "projected_total_points", "avg_points", "projected_avg_points",
               "injury_status", "injured", "percent_owned", "percent_started", "stats")


class ValidationError(Exception):
    pass


def encode(value):
    return json.dumps(value, ensure_ascii=False, allow_nan=False, indent=2) + "\n"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def check_config(c):
    bounds = {"league_id": (1, 2**64-1), "season": (2019, 65535),
              "team_id": (1, 2**32-1), "limit": (1, 2**32-1), "offset": (0, 2**32-1)}
    for key, (low, high) in bounds.items():
        if type(c.get(key)) is not int or not low <= c[key] <= high:
            raise ValidationError(f"invalid {key}")
    if c["offset"] + c["limit"] > 2**32-1:
        raise ValidationError("page offset plus limit overflows")
    if c.get("week") is not None and (type(c["week"]) is not int or not 1 <= c["week"] <= 2**32-1):
        raise ValidationError("invalid week")
    if c.get("player_id") is not None and (type(c["player_id"]) is not int or
            not -(2**63) <= c["player_id"] < 2**63 or c["player_id"] == 0):
        raise ValidationError("invalid player ID")


def cookies_from_env():
    pair = [os.environ.get("ESPN_S2"), os.environ.get("ESPN_SWID")]
    if pair == [None, None]:
        return {}
    if any(not v or any(ord(ch) <= 32 or ord(ch) >= 127 or ch == ";" for ch in v) for v in pair):
        raise ValidationError("private access requires both valid ESPN_S2 and ESPN_SWID cookie values")
    return {"espn_s2": pair[0], "SWID": pair[1]}


def trace(path, fantasy_filter):
    url = urlsplit(path)
    if url.scheme or url.netloc or url.fragment:
        raise ValidationError("recorder only accepts relative request paths")
    return {"method": "GET", "path": url.path,
            "query": parse_qs(url.query, keep_blank_values=True), "fantasy_filter": fantasy_filter}


def allowed_request(request, config):
    season = f"/apis/v3/games/ffl/seasons/{config['season']}"
    paths = {season, season + f"/segments/0/leagues/{config['league_id']}",
             f"/apis/v3/games/ffl/leagueHistory/{config['league_id']}"}
    query = request["query"]
    return (request["method"] == "GET" and request["path"] in paths and set(query) <= {"view", "scoringPeriodId", "seasonId"}
            and tuple(query.get("view", [])) in VIEWS
            and ("seasonId" not in query or query["seasonId"] == [str(config["season"])]))


def private_write(path, value):
    with path.open("x", encoding="utf-8") as stream:
        os.chmod(path, 0o600)
        stream.write(encode(value))


def output_directory(path):
    path = path.resolve()
    if path.is_relative_to(ROOT):
        raise ValidationError("capture/report folders must be outside the repository")
    path.mkdir(parents=True, mode=0o700, exist_ok=False)
    return path


def driver(mode, data, base=None):
    command = ["cargo", "run", "--locked", "--quiet", "--example", "validate", "--", mode]
    if base:
        command.append(base)
    environment = dict(os.environ)
    for key in ("ESPN_S2", "ESPN_SWID"):
        environment.pop(key, None)
    environment["NO_PROXY"] = environment.get("NO_PROXY", "") + ",127.0.0.1,localhost"
    try:
        run = subprocess.run(command, input=encode(data), text=True, capture_output=True,
                             cwd=ROOT, env=environment, timeout=600)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValidationError("cannot run the Rust validation driver; check Cargo and the toolchain") from error
    if run.returncode:
        return None
    try:
        return json.loads(run.stdout)
    except ValueError as error:
        raise ValidationError("invalid validation driver output") from error


def capture(config, output):
    import requests
    check_config(config)
    cookies = cookies_from_env()
    folder = output_directory(output)
    records = []
    started = time.time_ns() // 1_000_000

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *_args):
            pass

        def do_GET(self):
            try:
                fantasy_filter = self.headers.get("x-fantasy-filter")
                request = trace(self.path, json.loads(fantasy_filter) if fantasy_filter else None)
                if not allowed_request(request, config):
                    raise ValidationError("unexpected recorder request")
            except (ValidationError, ValueError):
                self.send_response(400)
                self.end_headers()
                return
            record = {"request": request, "received_unix_ms": time.time_ns() // 1_000_000}
            # Forward only this contract header and the caller's cookie pair.
            # Do not forward inbound cookies, authorization or arbitrary hosts.
            headers = {"x-fantasy-filter": fantasy_filter} if fantasy_filter else {}
            try:
                response = requests.get(UPSTREAM + self.path, headers=headers, cookies=cookies,
                                        timeout=30, allow_redirects=False)
                body = response.content
                status = response.status_code
                record.update(status=status, body_base64=base64.b64encode(body).decode("ascii"),
                              body_sha256=sha(body))
            except requests.RequestException as error:
                # The exception string can include proxy or credential details.
                record.update(status=None, network_error=type(error).__name__)
                body, status = b'{"error":"upstream network access failed"}', 502
            records.append(record)
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass

    server = HTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        context = driver("read", config, f"http://127.0.0.1:{server.server_port}/apis/v3/games/")
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=1)
    now = time.time_ns() // 1_000_000
    if context is not None:
        context["now_unix_ms"] = now
    status = capture_status(records, context)
    bundle = {"format_version": 1, "origin": "live_capture", "config": config,
              "credential_mode": "cookies_supplied" if cookies else "no_cookies",
              "started_unix_ms": started, "completed_unix_ms": now,
              "capture_status": status, "context": context, "requests": records}
    private_write(folder / "capture.json", bundle)
    report = {"status": status, "request_count": len(records),
              "credential_mode": bundle["credential_mode"],
              "parity": "not_run", "counts": context.get("counts") if context else None}
    private_write(folder / "capture-report.json", report)
    return report


def capture_status(records, context):
    if any(r.get("network_error") for r in records):
        return "network_error"
    last = records[-1].get("status") if records else None
    if last in (401, 403) or (context is None and len(records) > 1
            and records[-2].get("status") == 401 and last != 200):
        return "access_denied"
    if any(r.get("status") != 200 and r.get("status") != 401 for r in records):
        return "http_error"
    return "captured" if context is not None else "driver_failed"


def response_body(record):
    try:
        body = base64.b64decode(record["body_base64"], validate=True)
    except (KeyError, ValueError) as error:
        raise ValidationError("missing or malformed response bytes") from error
    if sha(body) != record.get("body_sha256"):
        raise ValidationError("response hash mismatch")
    return body


def replay_input(bundle):
    if bundle.get("format_version") != 1:
        raise ValidationError("unsupported capture format")
    check_config(bundle["config"])
    if bundle.get("capture_status") != "captured" or not bundle.get("context"):
        raise ValidationError("capture is incomplete; live access and parity are separate results")
    for record in bundle["requests"]:
        if not allowed_request(record["request"], bundle["config"]):
            raise ValidationError("unexpected captured request")
        response_body(record)
    good = [r for r in bundle["requests"] if r["status"] == 200]
    if len(good) != len(ROLES):
        raise ValidationError("capture does not contain the complete selected read sequence")
    result = {"config": bundle["config"], "context": bundle["context"]}
    for role, views, record in zip(ROLES, VIEWS, good):
        if tuple(record["request"]["query"].get("view", [])) != views:
            raise ValidationError(f"unexpected successful request for {role}")
        try:
            value = json.loads(response_body(record))
        except ValueError as error:
            raise ValidationError(f"non-JSON response for {role}") from error
        # Exactly the Rust/Python league transport's historical array unwrap;
        # season schedules are auxiliary resources and are not unwrapped.
        if role in ("league", "weekly", "free_agents", "cards") and isinstance(value, list):
            if not value:
                raise ValidationError("empty historical league wrapper")
            value = value[0]
        result[role] = value
    ctx, config = result["context"], result["config"]
    for record in (good[1], good[3], good[4], good[6]):
        if record["request"]["query"].get("scoringPeriodId") != [str(ctx["scoring_period"])]:
            raise ValidationError("captured week disagrees with replay context")
    matchup = good[1]["request"]["fantasy_filter"]["schedule"]["filterMatchupPeriodIds"]["value"]
    if matchup not in ([ctx["matchup_period"]], [str(ctx["matchup_period"])]):
        raise ValidationError("captured matchup disagrees with replay context")
    page = good[4]["request"]["fantasy_filter"]["players"]
    if page.get("limit") != config["limit"] or page.get("offset", 0) != config["offset"]:
        raise ValidationError("captured pagination disagrees with configuration")
    card_ids = good[7]["request"]["fantasy_filter"]["players"]["filterIds"]["value"]
    if card_ids != [ctx["player_id"]]:
        raise ValidationError("captured player card disagrees with replay context")
    return result


def reference_hashes(reference):
    return {str(p.relative_to(reference)): sha(p.read_bytes()) for p in sorted(reference.rglob("*")) if p.is_file()}


def selected_player(value):
    result = {k: copy.deepcopy(value[k]) for k in PLAYER_KEYS}
    for lines in result["stats"].values():
        for line in lines.values():
            line.pop("breakdown", None)
            line.pop("points_breakdown", None)
    return result


def python_oracle(bundle, source):
    import requests_mock
    from espn_api.base_league import BaseLeague
    from espn_api.football import League
    from espn_api.football.settings import Settings
    from espn_api.football.team import Team
    import generate_player_fixtures as players

    c, ctx = source["config"], source["context"]
    records = bundle["requests"]
    cursor = 0
    offset_difference = False

    def matcher(request):
        nonlocal cursor, offset_difference
        if cursor >= len(records):
            raise ValidationError("unexpected extra Python request")
        url = urlsplit(request.url)
        if url.scheme != "https" or url.netloc != "lm-api-reads.fantasy.espn.com":
            raise ValidationError("unexpected Python host")
        actual = trace(url.path + ("?" + url.query if url.query else ""),
                       json.loads(request.headers["x-fantasy-filter"]) if "x-fantasy-filter" in request.headers else None)
        actual["method"] = request.method
        record = records[cursor]
        expected = copy.deepcopy(record["request"])
        if expected["query"].get("view") == ["kona_player_info"] and c["offset"]:
            expected["fantasy_filter"]["players"].pop("offset", None)
            offset_difference = True
        if actual != expected:
            raise ValidationError(f"Python request differs at index {cursor}")
        cursor += 1
        return requests_mock.create_response(request, status_code=record["status"], content=response_body(record))

    def player(p):
        return selected_player(players.player_projection(p, []))

    def box_player(p, raw, ratings):
        result = players.free_agent_projection(p, raw, ratings, [])
        result["player"] = selected_player(result["player"])
        return result

    players.NOW = ctx["now_unix_ms"]
    os.environ["TZ"] = "UTC"
    if hasattr(time, "tzset"):
        time.tzset()
    with requests_mock.Mocker(real_http=False) as mock, patch("espn_api.football.box_player.datetime", players.FrozenDatetime):
        mock.add_matcher(matcher)
        league = League(c["league_id"], c["season"], fetch_league=False)
        data = BaseLeague._fetch_league(league, SettingsClass=Settings)
        # Explicit core load without Python's unrelated eager auxiliary calls.
        BaseLeague._fetch_teams(league, data, TeamClass=Team, pro_schedule={})
        team = next(t for t in league.teams if t.team_id == c["team_id"])
        snapshot = {"league": {"id":league.league_id, "season":league.year,
            "name":league.settings.name, "current_week":league.current_week,
            "current_matchup_period":league.currentMatchupPeriod},
            "team": {"id":team.team_id, "name":team.team_name, "roster":[player(p) for p in team.roster]}}
        boxes = league.box_scores(c["week"], {})
        ranks = players.ranking_map(source["weekly_ratings"])
        matchups = []
        for box, raw in zip(boxes, source["weekly"]["schedule"]):
            sides = {}
            for side in ("home", "away"):
                if side not in raw:
                    sides[side] = None
                else:
                    lineup = getattr(box, side + "_lineup")
                    entries = raw[side]["rosterForCurrentScoringPeriod"]["entries"]
                    sides[side] = {"team_id":raw[side]["teamId"], "score":getattr(box, side+"_score"),
                        "projected":getattr(box, side+"_projected"),
                        "lineup":[box_player(p, r, ranks) for p, r in zip(lineup, entries)]}
            matchups.append({"id":raw.get("id"), **sides, "matchup_type":box.matchup_type, "is_playoff":box.is_playoff})
        snapshot["weekly"] = {"season":c["season"], "scoring_period":ctx["scoring_period"],
                              "matchup_period":ctx["matchup_period"], "matchups":matchups}
        available = league.free_agents(week=ctx["scoring_period"], size=c["limit"])
        ranks = players.ranking_map(source["free_agents_ratings"])
        snapshot["free_agents"] = [box_player(p, raw, ranks) for p, raw in zip(available, source["free_agents"]["players"])]
        card = league.player_info(playerId=ctx["player_id"])
        cards = card if isinstance(card, list) else [card] if card is not None else []
        snapshot["cards"] = [{"player":player(p), "schedule":{str(period):{
            "opponent":game["team"], "date_unix_ms":round(game["date"].timestamp()*1000)}
            for period, game in p.schedule.items()}} for p in cards]
    if cursor != len(records):
        raise ValidationError("Python did not consume every captured request")
    return snapshot, {"matched_requests":cursor, "offset_extension_normalized":offset_difference}


def differences(actual, expected, path="$", limit=40):
    """Exact numeric equality, ordered lists; report paths without private values."""
    if limit <= 0:
        return []
    if isinstance(actual, dict) and isinstance(expected, dict):
        result = [path+"."+str(k)+": field missing" for k in sorted(actual.keys() ^ expected.keys())][:limit]
        for key in sorted(actual.keys() & expected.keys()):
            result += differences(actual[key], expected[key], path+"."+key, limit-len(result))
        return result
    if isinstance(actual, list) and isinstance(expected, list):
        if len(actual) != len(expected):
            return [path+": list length differs"]
        result = []
        for i, (a, e) in enumerate(zip(actual, expected)):
            result += differences(a, e, f"{path}[{i}]", limit-len(result))
        return result
    if type(actual) is bool or type(expected) is bool:
        return [] if type(actual) is type(expected) and actual == expected else [path+": value differs"]
    return [] if actual == expected else [path+": value differs"]


def verify(bundle, reference, output):
    folder = output_directory(output)
    report = {"status":"invalid_capture", "live_origin":bundle.get("origin") == "live_capture",
              "credential_mode":bundle.get("credential_mode"), "scope":"selected manager reads; not full football parity"}
    before = None
    try:
        source = replay_input(bundle)
        before = reference_hashes(reference)
        if not (reference / "espn_api/football/league.py").is_file():
            raise ValidationError("Python reference not found")
        sys.path.insert(0, str(reference.resolve()))
        sys.path.insert(0, str(ROOT / "scripts"))
        import espn_api
        if not Path(espn_api.__file__).resolve().is_relative_to(reference.resolve()):
            raise ValidationError("another Python reference is loaded; verify in a fresh process")
        actual = driver("project", source)
        if actual is None:
            report["status"] = "rust_decode_failed"
        else:
            report["status"] = "python_oracle_failed"
            expected, requests = python_oracle(bundle, source)
            diff = differences(actual, expected)
            report.update(status="parity_failed" if diff else "parity_passed", differences=diff,
                          request_contract=requests, counts=source["context"].get("counts"),
                          gaps=[] if actual["cards"] else ["requested player card absent; card fields not exercised"])
            private_write(folder / "rust.json", actual)
            private_write(folder / "python.json", expected)
    except Exception as error:
        # Keep private values and credentials out of the report/console. Request
        # mismatch index and integrity errors are our own sanitized diagnostics.
        report["error_type"] = type(error).__name__
        if report["status"] in ("parity_passed", "parity_failed"):
            report["status"] = "verification_failed"
        if isinstance(error, ValidationError):
            report["detail"] = str(error)
    finally:
        if before is not None:
            unchanged = before == reference_hashes(reference)
            report["reference"] = {"file_count":len(before), "unchanged":unchanged, "source_sha256":before}
            if not unchanged:
                report["status"] = "reference_changed"
    private_write(folder / "verification-report.json", report)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="mode", required=True)
    cap = sub.add_parser("capture", help="contact ESPN through a local Rust request recorder")
    cap.add_argument("league_id", type=int)
    cap.add_argument("season", type=int)
    cap.add_argument("team_id", type=int)
    cap.add_argument("--week", type=int)
    cap.add_argument("--limit", type=int, default=50)
    cap.add_argument("--offset", type=int, default=0)
    cap.add_argument("--player-id", type=int)
    cap.add_argument("--output", type=Path, required=True)
    replay = sub.add_parser("verify", help="compare the saved bytes without ESPN access")
    replay.add_argument("bundle", type=Path)
    replay.add_argument("--reference", type=Path, default=ROOT / "reference")
    replay.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        if args.mode == "capture":
            config = {k:getattr(args, k) for k in ("league_id", "season", "team_id", "week", "limit", "offset", "player_id")}
            report = capture(config, args.output)
            success = report["status"] == "captured"
        else:
            bundle = json.loads(args.bundle.read_text())
            report = verify(bundle, args.reference.resolve(), args.output)
            success = report["status"] == "parity_passed"
        print(encode({k:v for k,v in report.items() if k != "reference"}), end="")
        return 0 if success else 1
    except Exception as error:
        print(str(error) if isinstance(error, ValidationError) else
              f"validation setup failed ({type(error).__name__}); check paths and dependencies", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
