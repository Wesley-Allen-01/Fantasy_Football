"""Offline checks for the actual Rust-driver capture/replay workflow.

The response set is synthetic and never presented as fresh/live evidence.
Python reference comparison runs when the optional frozen checkout is available.
"""
import base64
import copy
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import requests
import requests_mock
import validate_live as live

FIXTURES = live.ROOT / "tests/fixtures"
CONFIG = {"league_id":123, "season":2024, "team_id":1, "week":None,
          "limit":3, "offset":0, "player_id":7002}


def fixture(name):
    return json.loads((FIXTURES / name).read_text())


def responses():
    league = fixture("football_2018_league.json")
    league.update(id=123, seasonId=2024, scoringPeriodId=7)
    league["status"].update(currentMatchupPeriod=4, firstScoringPeriod=1,
        finalScoringPeriod=18, latestScoringPeriod=7, previousSeasons=[2023])
    league["settings"]["scheduleSettings"]["matchupPeriods"] = {"4":[7]}
    # Stale-season roster statistics deliberately remain stale, testing filters.
    weekly = fixture("weekly/synthetic_2024_input.json")["cases"][0]
    available = fixture("players/free_agents_input.json")["cases"][0]
    cards = fixture("players/synthetic_2024_cards_input.json")
    cards["cards"]["players"] = [p for p in cards["cards"]["players"] if p["id"] == 7002]
    return [league, weekly["box_scores"], weekly["pro_schedule"], weekly["positional_ratings"],
            available["players"], available["pro_schedule"], available["positional_ratings"],
            cards["cards"], cards["pro_schedule"]]


def upstream_matcher(payloads):
    cursor = 0
    def match(request):
        nonlocal cursor
        if not request.url.startswith(live.UPSTREAM + "/apis/v3/games/"):
            return None
        if cursor >= len(payloads):
            raise AssertionError("unexpected extra capture request")
        response = requests_mock.create_response(request, status_code=200,
            content=live.encode(payloads[cursor]).encode())
        cursor += 1
        return response
    return match


class CaptureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.folder = tempfile.TemporaryDirectory(prefix="espn-validation-tests-")
        cls.root = Path(cls.folder.name)
        with requests_mock.Mocker(real_http=False) as mock, patch.dict(os.environ, {"ESPN_S2":"test-secret", "ESPN_SWID":"test-swid"}):
            mock.add_matcher(upstream_matcher(responses()))
            cls.report = live.capture(CONFIG, cls.root / "capture")
        cls.bundle = json.loads((cls.root / "capture/capture.json").read_text())
        cls.bundle["origin"] = "synthetic_test"
        if cls.report["status"] != "captured":
            raise AssertionError(f"fixture capture failed: {cls.report}")

    @classmethod
    def tearDownClass(cls):
        cls.folder.cleanup()

    def test_real_client_capture_preserves_exact_responses_without_credentials(self):
        self.assertEqual(len(self.bundle["requests"]), 9)
        self.assertEqual(self.bundle["credential_mode"], "cookies_supplied")
        encoded = live.encode(self.bundle)
        self.assertNotIn("test-secret", encoded)
        self.assertNotIn("test-swid", encoded)
        self.assertEqual((self.root / "capture/capture.json").stat().st_mode & 0o777, 0o600)
        for record, original in zip(self.bundle["requests"], responses()):
            self.assertEqual(json.loads(live.response_body(record)), original)

    def test_rust_replay_against_existing_python_goldens(self):
        bundle = copy.deepcopy(self.bundle)
        bundle["context"]["now_unix_ms"] = 1728230400000
        output = live.driver("project", live.replay_input(bundle))
        self.assertIsNotNone(output)
        expected = fixture("players/free_agents_expected.json")["cases"][0]["players"]
        for actual, golden in zip(output["free_agents"], expected):
            golden["player"] = live.selected_player(golden["player"])
            self.assertEqual(live.differences(actual, golden), [])
        self.assertEqual(len(output["free_agents"]), len(expected))
        self.assertEqual(output["team"]["id"], 1)
        self.assertEqual(output["cards"][0]["player"]["id"], 7002)

    @unittest.skipUnless((live.ROOT / "reference/espn_api/football/league.py").is_file(), "optional frozen Python reference is unavailable")
    def test_complete_capture_and_closed_python_oracle(self):
        report = live.verify(self.bundle, live.ROOT / "reference", self.root / "verify")
        self.assertEqual(report["status"], "parity_passed", report)
        self.assertEqual(report["request_contract"]["matched_requests"], 9)
        self.assertTrue(report["reference"]["unchanged"])
        self.assertFalse(report["live_origin"])

    @unittest.skipUnless((live.ROOT / "reference/espn_api/football/league.py").is_file(), "optional frozen Python reference is unavailable")
    def test_nonzero_offset_is_the_only_normalized_request_extension(self):
        config = {**CONFIG, "offset":3}
        with requests_mock.Mocker(real_http=False) as mock:
            mock.add_matcher(upstream_matcher(responses()))
            live.capture(config, self.root / "offset")
        bundle = json.loads((self.root / "offset/capture.json").read_text())
        bundle["origin"] = "synthetic_test"
        report = live.verify(bundle, live.ROOT / "reference", self.root / "offset-verify")
        self.assertEqual(report["status"], "parity_passed", report)
        self.assertTrue(report["request_contract"]["offset_extension_normalized"])

    @unittest.skipUnless((live.ROOT / "reference/espn_api/football/league.py").is_file(), "optional frozen Python reference is unavailable")
    def test_oracle_rejects_request_filter_drift(self):
        bundle = copy.deepcopy(self.bundle)
        bundle["requests"][1]["request"]["fantasy_filter"]["unexpected"] = True
        with self.assertRaises(live.ValidationError):
            live.python_oracle(bundle, live.replay_input(bundle))

    def test_hash_tampering_is_rejected(self):
        bundle = copy.deepcopy(self.bundle)
        bundle["requests"][0]["body_base64"] = base64.b64encode(b"{}").decode()
        with self.assertRaisesRegex(live.ValidationError, "hash mismatch"):
            live.replay_input(bundle)

    def test_replay_rejects_missing_requests_and_context_drift(self):
        bundle = copy.deepcopy(self.bundle)
        bundle["requests"].pop()
        with self.assertRaises(live.ValidationError):
            live.replay_input(bundle)
        bundle = copy.deepcopy(self.bundle)
        bundle["context"]["scoring_period"] += 1
        with self.assertRaisesRegex(live.ValidationError, "week disagrees"):
            live.replay_input(bundle)

    def test_existing_output_is_not_overwritten(self):
        with self.assertRaises(FileExistsError):
            live.output_directory(self.root / "capture")

    def test_network_failure_is_not_parity_failure_and_does_not_leak(self):
        with patch("requests.get", side_effect=requests.exceptions.ProxyError("test-secret")):
            report = live.capture(CONFIG, self.root / "blocked")
        self.assertEqual(report["status"], "network_error")
        self.assertEqual(report["parity"], "not_run")
        raw = (self.root / "blocked/capture.json").read_text()
        self.assertNotIn("test-secret", raw)
        failed = json.loads(raw)
        with self.assertRaisesRegex(live.ValidationError, "incomplete"):
            live.replay_input(failed)

    def test_access_denial_is_recorded_without_parity_claim(self):
        with requests_mock.Mocker(real_http=False) as mock:
            mock.get(requests_mock.ANY, status_code=401, json={})
            report = live.capture(CONFIG, self.root / "denied")
        self.assertEqual(report["status"], "access_denied")
        self.assertEqual(report["request_count"], 2)

    @unittest.skipUnless((live.ROOT / "reference/espn_api/football/league.py").is_file(), "optional frozen Python reference is unavailable")
    def test_successful_historical_fallback_replays_the_exact_request_sequence(self):
        original = upstream_matcher(responses())
        first = True
        def fallback(request):
            nonlocal first
            if first:
                first = False
                return requests_mock.create_response(request, status_code=401, content=b"{}")
            return original(request)
        with requests_mock.Mocker(real_http=False) as mock:
            mock.add_matcher(fallback)
            captured = live.capture(CONFIG, self.root / "fallback")
        self.assertEqual(captured["status"], "captured")
        bundle = json.loads((self.root / "fallback/capture.json").read_text())
        bundle["origin"] = "synthetic_test"
        result = live.verify(bundle, live.ROOT / "reference", self.root / "fallback-verify")
        self.assertEqual(result["status"], "parity_passed", result)
        self.assertEqual(result["request_contract"]["matched_requests"], 10)

    def test_invalid_response_is_not_reported_as_a_successful_capture(self):
        with requests_mock.Mocker(real_http=False) as mock:
            mock.get(requests_mock.ANY, status_code=200, text="not JSON")
            result = live.capture(CONFIG, self.root / "invalid-response")
        self.assertEqual(result["status"], "driver_failed")
        self.assertEqual(result["parity"], "not_run")


class ContractTests(unittest.TestCase):
    def test_successful_route_fallback_is_not_reported_as_access_denial(self):
        records = [{"status":401}, {"status":200}, {"status":200}]
        self.assertEqual(live.capture_status(records, None), "driver_failed")
        self.assertEqual(live.capture_status(records, {}), "captured")

    def test_partial_credentials_rejected(self):
        with patch.dict(os.environ, {"ESPN_S2":"secret"}, clear=True):
            with self.assertRaises(live.ValidationError):
                live.cookies_from_env()

    def test_invalid_options_fail_before_network(self):
        for field, value in (("week",0), ("team_id",0), ("limit",0), ("offset",2**32), ("season",2018), ("player_id",0)):
            with self.subTest(field=field), self.assertRaises(live.ValidationError):
                live.check_config({**CONFIG, field:value})

    def test_recorder_rejects_other_hosts_paths_and_methods(self):
        with self.assertRaises(live.ValidationError):
            live.trace("https://other.example/test", None)
        request = live.trace("/apis/v3/games/ffl/seasons/2024?view=proTeamSchedules_wl", None)
        self.assertTrue(live.allowed_request(request, CONFIG))
        self.assertFalse(live.allowed_request({**request, "path":"/apis/v3/games/ffl/seasons/2025"}, CONFIG))
        self.assertFalse(live.allowed_request({**request, "method":"POST"}, CONFIG))

    def test_repository_capture_path_is_rejected(self):
        with self.assertRaisesRegex(live.ValidationError, "outside"):
            live.output_directory(live.ROOT / "reference/capture-forbidden")

    def test_exact_comparison_distinguishes_missing_zero_and_booleans(self):
        self.assertEqual(live.differences({"points":0.0}, {"points":0}), [])
        self.assertTrue(live.differences({"points":None}, {"points":0}))
        self.assertTrue(live.differences({"points":True}, {"points":1}))
        self.assertTrue(live.differences([1,2], [2,1]))
        self.assertTrue(live.differences(1.001, 1.002))


if __name__ == "__main__":
    unittest.main()
