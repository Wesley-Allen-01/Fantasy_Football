# Validate your league against current ESPN data

The `capture` command runs the real Rust client through a local recorder. The
`verify` command runs Python and Rust on the saved responses, without contacting
ESPN. This avoids false differences caused by live scores changing between reads.
It is a selected comparison, not a claim of complete football parity.

Run these commands from the updated project folder on a machine that can reach
ESPN. This cloud environment currently blocks ESPN before any league response.
Rust 1.85+ and Python 3.10+ are required. Prepare an isolated Python environment:

```sh
python3 -m venv /tmp/espn-validation-venv
/tmp/espn-validation-venv/bin/python -m pip install -r scripts/requirements-validation.txt
```

Capture your league and team 1 for 2026:

```sh
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-validation-venv/bin/python scripts/validate_live.py capture \
  394172912 2026 1 --output /tmp/espn-league-394172912-capture
```

If private access is needed, supply both `ESPN_S2` and `ESPN_SWID` as environment
variables before running. Do not put cookies in command arguments, source files
or Git. The recorder does not save cookies or forward arbitrary inbound headers;
the Rust driver does not receive the credential variables. Credential presence
does not itself establish successful private-league access.

The capture includes a league load, weekly scores/lineups, one free-agent/waiver
page and one player card. It selects the first roster player for the card unless
you supply `--player-id ID`; negative defense IDs are supported. Optional `--week
N`, `--limit N` and `--offset N` control the read. A future week follows the
existing box-score fallback, and both the lineup and available-player query use
the effective week. It never loops through pages or submits ESPN changes.

Captures must be saved **outside the repository**. Their folder must be new;
existing results are never overwritten. Raw responses can contain league member
information, so local files use restrictive permissions. The tool does not
upload them. A failed capture still produces `capture.json` and a small
`capture-report.json`, distinguishing network/access/HTTP/driver failures from
parity. A capture failure returns a nonzero exit status.

After a successful capture, compare it against the frozen reference:

```sh
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-validation-venv/bin/python scripts/validate_live.py verify \
  /tmp/espn-league-394172912-capture/capture.json \
  --reference reference --output /tmp/espn-league-394172912-verification
```

If the frozen Python checkout is elsewhere, use its actual path for `--reference`.
It must contain `espn_api/football/league.py`. The Rust library itself and capture
do not require the Python reference. Verification does not change it, and checks
file hashes before and after, including failed comparisons.

The verification folder contains `verification-report.json` and, when both
converters succeed, `rust.json` and `python.json`. A passing report says
`parity_passed`; a failed report distinguishes invalid/incomplete capture, Rust
decoding, Python oracle and semantic differences. Reports show difference paths,
not private values. No numeric tolerance hides changes in points. Statuses and
console output omit credentials and raw response/exception contents.

The comparison covers:

- The selected team's full roster identity, eligibility, injury/ownership,
  actual/projected season totals/averages and per-period availability/points.
- Every returned weekly matchup and lineup: team scores/projections, player
  points, NFL opponent/rank/kickoff and the reference's 0/100 time heuristic.
- One available-player page and one ID-based card, including its NFL schedule.
- Ordered request paths, queries and decoded filters against real Python calls.

Python's unrelated eager directory/draft loads are explicitly omitted. Missing
weekly sides/opponents/ranks normalize to the documented Rust representation.
Nonzero page offsets are an explicit Rust extension: only that field is removed
for Python request comparison, and the report names the correction. A successful
offset request does not prove ESPN honored it: capture separate pages and inspect
their IDs; pages can legitimately repeat. Raw/applied stat breakdowns, directory
name lookup, historical rosters and every uncommon configuration are outside
this comparison. An absent player card is reported as a coverage gap.

Automated offline checks for this workflow:

```sh
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-validation-venv/bin/python -m unittest discover \
  -s scripts -p 'test_validate_live.py' -v
```

The tests record simulated responses through the actual Rust client and compare
existing Python goldens. Additional closed Python-oracle tests run only when the
optional frozen reference exists; CI without it explicitly skips those tests.
Simulated captures are not evidence of successful current-season/private access.
