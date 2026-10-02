# Football compatibility ledger

This first slice preserves league metadata, football settings, team and roster identity, actual/projected fantasy statistics, ID-linked schedules, completed-season standings and remote scoreboards. It is a read-only foundation. Draft enrichment, NFL schedules, box scores, free agents, transactions, weekly standings and power rankings remain deferred.

## Preserved behaviors

| Behavior | Reference location | Evidence |
| --- | --- | --- |
| Seasons before 2018 start on leagueHistory; array responses unwrap to the league object | `reference/espn_api/requests/espn_requests.py` | Transport tests; 2015 oracle/input fixture |
| Initial league views include mTeam, mRoster, mMatchup, mSettings, mStandings | `get_league` in the same file | Transport/client request assertions |
| On 401, try the alternate route and retain the new route only after successful JSON decoding | `checkRequestStatus` in the same file | Transport regression tests |
| NFL week comes from status.latestScoringPeriod, separately from scoring/current matchup periods | `reference/espn_api/football/league.py::_fetch_league` | Both Python goldens; absence is represented as None in Rust |
| Modern current week is capped at finalScoringPeriod; older seasons use scoringPeriodId directly | `reference/espn_api/base_league.py::_fetch_league` | 2015 and 2018 Python goldens |
| Team names use name or location + nickname; teams sort by ID; rankFinal=0 falls back to rankCalculatedFinal | `reference/espn_api/football/team.py`, `base_league.py` | Python goldens for all 18 fixture teams |
| Points against and fantasy stat totals/averages use Python rounding to two decimal places | `football/team.py`, `football/player.py` | Exact golden comparison with correctly rounded JSON float parsing, no blanket floating tolerance |
| Player identities include negative defense IDs; primary position follows eligible-slot order | `football/player.py` | Representative offensive players and defense entries |
| Only matching-season stats are used; split type 2 is ignored; actual source 0 and projections remain separate | `football/player.py` | Stat projections from both reference seasons |
| Schedule order and scores preserve the response; team-relative outcomes and margin of victory derive from the matchup | `football/team.py::_fetch_schedule`, `football/league.py::_fetch_teams` | All 146 fixture schedule rows |
| Standings use nonzero final standing, otherwise playoff seed, with stable ordering | `football/league.py::standings` | Python standings order for both seasons |
| Scoreboard requests mMatchupScore and filters matchupPeriodId locally; default period follows current_week | `football/league.py::scoreboard` | Python period 1 and current-week goldens; client tests |
| Scoreboards need no winner metadata and skip irrelevant periods before decoding matchup sides | `football/league.py::scoreboard` | Targeted regressions for absent winners and incomplete rows in other periods |
| Missing/null statSourceId updates active_status while storing projected points | `football/player.py` | Targeted regression for the reference's separate falsiness and source-equality checks |

## Deliberate differences

| Python behavior | Rust decision | Reason / verification |
| --- | --- | --- |
| Construction eagerly loads league, player pool, NFL schedules and draft | Client/handle construction does no I/O; explicit fetch loads the first-slice views | Async loading and bounded scope; client tests check explicit requests |
| requests follows HTTP redirects automatically | Disable redirects and return an HTTP error | Prevent credentials from following redirects; transport tests |
| A 404 extension containing the substring communication becomes empty topics | Match the exact communication path segment | Avoid hiding unrelated missing endpoints; transport tests |
| Explicit scoreboard period 0 defaults through Python falsiness | Reject explicit zero periods; only omitted periods default, including a loaded preseason current_week of zero | Typed input validation and preseason regression; client tests |
| Python accepts a league payload whose ID differs from the requested league | Reject mismatched league/season IDs and inconsistent roster player IDs | Detect wrong responses before publishing a snapshot; fixture requests use actual payload IDs |
| Linked team objects form cycles and can mutate shared state | Owned snapshots link teams by typed ID | Predictable ownership; resolving links is explicit |
| Bye weeks point a team's schedule back to itself with margin 0 | Schedule opponent and margin are None for byes | No fabricated opponent; golden normalization records this one difference |
| Missing scoreboard sides synthesize ID 0 and score 0 | Missing side and score are None | Distinguishes absence from a team scoring zero; golden normalization records this difference |
| Settings maps are shared and mutated during construction | Each snapshot owns scoring rules | Prevents one league changing another league's rules; model regression tests |
| Roster counts zip dictionary values against a separate position list | Parse counts using their explicit numeric slot keys | Correct sparse/out-of-order maps; model regression tests |
| A scoring override of zero falls through to the default via `or` | An explicit zero override is retained | Zero has scoring meaning; model regression tests |
| Refresh falls back to BaseSettings and may partially replace state | Decode a full football snapshot before replacement | Football settings retained and failed refresh leaves the prior snapshot; client tests |
| Margin of victory reads opponent scores at the same vector index | Compute margin from the two sides of the actual matchup | Handles uneven/irregular schedules; model regression tests |
| Raw stat labels can collide (e.g. different passing-yard IDs share one label) | Retain typed numeric stat IDs in raw/applied breakdowns | Prevents lossy merging; breakdown label parity is deliberately outside these goldens |
| Unknown mapped slot/team IDs raise KeyError | Preserve numeric IDs; optional label lookup | Allows ESPN to add identifiers without breaking data loading; model tests |

No correction to transactions or draft refresh is claimed in this slice: those features are deferred.

## Oracle and fixtures

`scripts/generate_parity_fixtures.py` imports the unchanged Python package and constructs real Python League instances. `requests-mock` intercepts every request, with real HTTP disabled and exact query strings. Unregistered requests fail. The player-pool, NFL-schedule and draft endpoints have explicit empty responses because those features are outside this slice; roster identity and statistics still come from the actual league payloads.

Before writing each golden, the generator compares its selected semantic projection from the original full reference league fixture with the projection from the compact input. The compact input retains every team and schedule row but selects the first roster entry plus a defense or quarterback when available, and removes large rankings/news/stat-breakdown maps and schedule lineups. Stat selection flags, original totals/averages, ordering, and all season records remain intact. Original source SHA-256 values are in `tests/fixtures/provenance.json`.

Goldens compare selected semantic fields, not full Rust serialization. Slot and pro-team numeric IDs normalize to the Python labels. Stats normalize to per-period actual/projected totals and averages. The two documented absent-side/bye corrections normalize explicitly; no other unexplained difference should be hidden in the oracle. The compact checked-in fixtures total approximately 340 KiB instead of copying the original multi-megabyte league payloads.

Generate or verify without modifying reference content:

```bash
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-investigation-venv/bin/python scripts/generate_parity_fixtures.py
PYTHONDONTWRITEBYTECODE=1 /tmp/espn-investigation-venv/bin/python scripts/generate_parity_fixtures.py --check
source /workspace/.dev-tools/activate.sh
cargo test --test parity
```

Use `--reference /path/to/reference` or `ESPN_PYTHON_REFERENCE` to locate another frozen checkout. The generator sets `sys.dont_write_bytecode` as an additional safeguard. The Rust parity test can optionally compare the full original input using `ESPN_PYTHON_REFERENCE=/path/to/reference cargo test --test parity`; its projection selects the same roster IDs as the checked-in golden.

## Limits

The two old fixture seasons give concrete historical parity evidence, not a guarantee about every current ESPN response. The current-season/private-league release gate remains later work. Raw and applied stat breakdowns, injury/ownership fields, member records, and complete roster coverage are not asserted by these selected goldens and need additional targeted coverage. Transport tests establish request behavior separately from fixture/model parity. No live ESPN request is needed for the offline suite.
