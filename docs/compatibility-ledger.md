# Football compatibility ledger

The foundation preserves league metadata, football settings, team and roster identity, actual/projected fantasy statistics, ID-linked schedules, completed-season standings and remote scoreboards. The weekly slice adds box scores, lineups, NFL schedule context and opponent positional rankings. Player search adds free-agent pages, player cards and a season directory. Draft enrichment, historical roster loading, typed transactions, weekly standings and power rankings remain deferred.

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

The two old league fixture seasons give concrete historical parity evidence, not a guarantee about every current ESPN response. The current-season/private-league release gate remains later work. The foundation goldens do not assert raw/applied stat breakdowns, injury/ownership fields, member records or complete roster coverage; additional targeted coverage remains necessary. Weekly breakdown coverage is described below. Transport tests establish request behavior separately from fixture/model parity. No live ESPN request is needed for the offline suite.

## Weekly matchup slice

| Preserved behavior | Evidence |
| --- | --- |
| HTTP box scores require season 2019 or later; default and future weeks use loaded current week/current matchup, while explicit available weeks map through matchup_periods | Closed-mock real Python League.box_scores request traces; request and client tests |
| Main request uses repeated mMatchupScore/mScoreboard views, scoringPeriodId and schedule.filterMatchupPeriodIds; schedules are season-level; ratings are league-level with the chosen week | Python request trace compared against Rust requests, including ordering and mapped-key string versus default numeric filter values |
| totalPointsLive overrides totalPoints; live team projection is used when available, otherwise sum player projections excluding bench/IR | Frozen-time Python goldens and targeted model tests |
| Preserve actual/projected points and raw/applied breakdowns per selected week, lineup order, scoring rounding and playoff tier | Six synthetic modern cases and one complete historical matchup with all 30 lineup entries; model tests |
| Historical NFL team uses first matching-week actual nonzero proTeamId, then explicit history, then current player team; projections never update history | Python goldens and model tests for stale-season/split evidence, stat filtering and cache updates |
| First NFL game in the scoring period determines opponent and kickoff; positional rank uses default position; opponent is present only when the position map exists | Synthetic schedules/rankings in frozen-time goldens; model regressions |
| game_played is 100 strictly after kickoff plus three hours, 0 otherwise; missing schedule means bye and the Python default 100 | Fixed epoch-millisecond clock, strict boundary and overflow regressions |

| Deliberate difference | Rust behavior / evidence |
| --- | --- |
| Python's explicit week 0 defaults through falsiness | Explicit zero is rejected before weekly requests; omitted preseason current week 0 remains allowed |
| Missing fantasy side synthesizes zero score/projection and an empty lineup | Whole BoxTeam side is None, normalized explicitly in the oracle |
| Missing rank uses sentinel 0 | Missing rank is None; an explicit upstream rank 0 is retained |
| Caller cache has no season boundary and may partially mutate on failed parsing | PlayerTeamHistory is scoped to a season; complete successful conversion commits staged evidence atomically |
| Season-level GET reuses the league status handler, which can attempt a league-route fallback on 401 | Season-level reads return Http for non-200 responses and never mutate league routing; mocked error and array-response regressions |
| Recursive player identity/metadata lookup accepts inconsistent wrapper identities | Known wrapper paths, nested player precedence, explicit metadata fallback and identity validation; conflicting IDs are rejected |
| Naive local game datetime and ambient now | Raw kickoff epoch milliseconds and an explicit conversion clock; public client uses system time |

`scripts/generate_box_score_fixtures.py` generates six checked-in files in `tests/fixtures/weekly/`, with real HTTP disabled and a fixed UTC clock. The 2018 source fixture is used only for direct model characterization: the supported-season HTTP gate is never bypassed by the public client. Its selected first matchup includes all 30 lineup entries, and full-versus-compact Python projections must match. Modern cases are synthetic, not fresh ESPN captures. Python and Rust comparisons use exact binary numeric values without an epsilon.

The provenance explicitly lists duplicate Python stat-label aliases excluded from label-level comparison. Rust retains numeric-ID breakdowns, including these aliases; targeted numeric-ID model tests cover their preservation. Historical fixture compaction drops those ambiguous alias fields rather than asserting Python's lossy overwrite as the desired Rust behavior.

Verify the generator with `PYTHONDONTWRITEBYTECODE=1 python scripts/generate_box_score_fixtures.py --reference /path/to/reference --check`. Enable original full-input Rust comparisons with `ESPN_PYTHON_REFERENCE=/path/to/reference cargo test --locked --all-targets`. Private/current-season live access remains pending; the managed cloud only allows package-manager hosts, so ESPN has not been reached in this session.

## Free agents and player lookup

| Preserved behavior | Evidence |
| --- | --- |
| FA requests require 2019 or later, default to current_week/50, include FREEAGENT and WAIVERS, retain ownership/draft ordering, and do not clamp supplied future weeks | Real Python closed-mock request traces, exact Rust trace comparison and boundary tests |
| FA reads fetch player data, NFL schedule, then positional ratings, including a valid empty result | Python request traces and Rust request/client tests |
| Cards use kona_playercard/filterIds and final scoring period with season additionalValue strings | Scalar/multi/empty-result Python request traces; bounded batch tests |
| Directory is season-level /players, view players_wl, filterActive=true | Real Python request trace and raw-array request tests |
| Weekly FA identity, eligibility, injury, ownership, actual/projected stats, NFL opponent/rank and clock semantics follow BoxPlayer | Synthetic modern Python FA goldens; frozen UTC time; model tests |
| Card season stats and current-NFL-team first-game schedules, injury/ownership/eligibility and wrapper metadata | Compact/full original 2019 card and synthetic card goldens; raw wrapper equality |
| Positional rank falls back to known wrapper ratings['0'].positionalRanking; absent card schedule settings/proTeams means an empty schedule | Captured original card exposed rank fallback; explicit default-path tests without weakening weekly parsing |

| Deliberate difference | Rust behavior / evidence |
| --- | --- |
| Player return type varies between Player/list/None | Single-ID query returns Option<PlayerCard>; multi-ID/name queries always return a vector |
| Eager constructor loads the full name map; duplicate names select first ID | Explicit directory read; exact case-sensitive queries return all distinct matching IDs in response order; unknown names skip cards and schedules |
| Python position_id=0 is ignored by falsiness | Typed numeric slots preserve QB/0; oracle request comparison accounts only for this explicit filter correction |
| FA helper exposes one limit without offset pagination | Optional nonzero offset extension; advisory next_offset; no automatic looping or projection sorting; repeated/short/full/empty pages tested |
| Empty ID list still makes card/schedule requests; large direct card list is unbounded | Empty input performs no requests, stable input deduplication and batches of at most 40; publish results only after every batch and schedule succeeds |
| Recursive IDs or duplicate/inconsistent returned identities may be accepted | Known wrapper paths, identity validation, duplicate-result rejection and only-requested-ID check; negative defense IDs remain supported |
| Empty card NFL game arrays can raise on first element indexing | Empty period means no scheduled game; nonempty selected first game stays strict |
| Invalid zero inputs/overflow may reach ESPN | Explicit zero week/limit/ID, blank name and overflowing offsets are configuration errors before query requests |

`scripts/generate_player_fixtures.py` emits nine JSON/provenance files with closed HTTP mocks and a fixed UTC clock. Four player parity tests compare synthetic modern FA/cards, directory/name normalization, and compact/full original 2019 player-card data. Compaction must preserve the Python projection before output. Duplicate-label stat aliases are excluded specifically from label comparison, with numeric IDs retained and tested in Rust; exact binary numeric comparisons use no blanket epsilon. Raw card transactions are retained unchanged, not parsed as typed transaction history. The card auxiliary defaults accept absent settings/proTeams and empty arrays/objects, while null or malformed values remain errors.

Modern FA/card and supplemental NFL schedules are synthetic; the actual card fixture is 2019. No fresh current-season/private ESPN response is claimed. Offset support and service behavior must still be confirmed live. Player searches never replace the loaded snapshot; no cache, background refresh, account login, lineup submission or waiver recommendation is introduced.
