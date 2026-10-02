These fixtures characterize the unchanged Python football `BoxScore` and `BoxPlayer` models under a frozen UTC clock. They are not captured modern ESPN responses.

- `synthetic_2024_input.json` contains six explicitly synthetic cases: starter projection fallback excluding bench/IR, live-score precedence, exact kickoff-plus-three-hour boundary and one millisecond later, optional opponent rankings including present rank zero, missing sides/byes, actual historical-team correction, and projected-only history fallback without updating history. A stale-season actual entry also demonstrates Python's team-evidence search versus season/split filtering of points.
- `synthetic_2024_expected.json` comes from real Python `BoxScore`/`BoxPlayer` construction with those inputs.
- `historical_2018_input.json` selects the first matchup and all 30 lineup entries from the supplied 2018 box-score fixture. The generator removes large unrelated lineups/rankings/news fields. It proves that the selected full-input Python projection equals the compact-input projection before writing either golden. This is direct model characterization: Python `League.box_scores` rejects seasons before 2019. No captured 2018 NFL schedule or positional rankings were supplied, so the historical oracle uses explicit empty auxiliary responses.
- `python_requests.json` records actual Python `League.box_scores` calls with closed mocks for an explicit week, default week, future-week fallback, and the historical-season gate. Requests include their ordered paths, repeated query values, and parsed fantasy filters. Auxiliary GETs run in Python order. Real HTTP is disabled and unregistered requests fail. These fixtures complement the Rust request tests.
- `provenance.json` records the original source hash, timezone and normalizations.

Missing sides/opponents/dates/ranks normalize to null. Present rank zero remains zero. Missing NFL schedule retains Python's bye/game-played behavior. Numeric stat IDs remain authoritative in Rust; Python's duplicate label aliases are excluded from label comparison because Python overwrites distinct IDs with the same label. The explicit excluded labels are listed in provenance. Synthetic raw/applied comparisons use distinct touchdown/conversion stat IDs and remain fully asserted. No floating-point tolerance is used.

Regenerate or verify:

```bash
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-investigation-venv/bin/python scripts/generate_box_score_fixtures.py
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-investigation-venv/bin/python scripts/generate_box_score_fixtures.py --check
source /workspace/.dev-tools/activate.sh
cargo test --test weekly_parity
ESPN_PYTHON_REFERENCE=/workspace/Fantasy_Football/reference cargo test --test weekly_parity
```

The Python script accepts `--reference` or `ESPN_PYTHON_REFERENCE` for another frozen reference checkout. Dependencies are `requests` and `requests-mock`; importing the source needs no editable installation. The optional Rust environment variable additionally tests the same selected projection directly from the full 43 MB source fixture. Bytecode is suppressed in both the command and generator.

Current/private-league HTTP validation, arbitrary ESPN schema coverage, unknown mapped labels, complete historical player-team recovery without caller history, and exact live game progress are not established by these fixtures. The 0/100 game-played field is Python's strict three-hour heuristic.
