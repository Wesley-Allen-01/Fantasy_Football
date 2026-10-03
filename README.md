# ESPN Fantasy Football in Rust

An asynchronous, read-only Rust migration of the football API in `reference/`, based on Python `espn-api` 0.46.0. The reference is kept unchanged. League foundation, weekly matchups, free-agent pages and player lookup are implemented; complete football package parity remains in progress.

## Implemented

- Reusable HTTP client, paired private-league cookies, historical league routes and stateful endpoint fallback.
- Explicit league loading and atomic refresh, without network requests during construction.
- Football settings and scoring metadata, teams/members/owners, rosters and actual/projected season statistics.
- ID-linked schedules, missing-side/bye handling, team lookup and ESPN standings.
- Remote scoreboards with the Python request filters and default period behavior.
- Weekly box scores and both lineups, live score precedence, starter projection fallback, NFL schedules and opponent positional rankings.
- Explicit season-scoped historical NFL team evidence for traded players, with atomic updates after a complete successful parse.
- Free-agent and waiver pages with numeric eligibility filters, ownership ordering, injury/ownership metadata and weekly statistics.
- Player cards by ID, an explicitly loaded active-player directory, and exact-name lookup preserving duplicate names.
- Terminal examples for inspecting weekly matchups, comparing available players and looking up detailed player cards.
- Offline Python-derived comparison fixtures and an optional comparison against full original payloads.

Historical roster loading, draft/history/reporting, weekly standings and power rankings are later milestones. Submitting lineups, claims or trades is outside the reference's read-only API scope. See the [migration plan](docs/espn-rust-implementation-plan.md) and [compatibility ledger](docs/compatibility-ledger.md) for preserved behaviors, intentional corrections and evidence gaps.

## Use

```rust,no_run
use espn_fantasy_football::{Client, LeagueId, MatchupPeriod, Season, TeamId};

# async fn example() -> espn_fantasy_football::Result<()> {
let client = Client::builder().build()?;
let mut league = client.league(LeagueId(123456), Season(2026))?;
let snapshot = league.fetch().await?;

for team in snapshot.standings() {
    println!("{}: {} wins", team.name, team.wins);
}
if let Some(team) = snapshot.team(TeamId(1)) {
    println!("{} has {} roster entries", team.name, team.roster.len());
}

let scoreboards = league.scoreboard(Some(MatchupPeriod(1))).await?;
println!("{} matchups", scoreboards.len());
# Ok(())
# }
```

For a private league, pass `Credentials::new(espn_s2, swid)?` to `Client::builder().credentials(...)`. The client sends both `espn_s2` and `SWID`; credential Debug output is redacted. Client construction and parsing perform no HTTP requests. A basic fetch issues only the initial league views; professional player-directory, professional schedule and draft enrichment are deferred.

The client defaults to a 30-second timeout per HTTP request. A league 401 triggers one alternate-route request; a failed fallback retains its original route. Mutable league handles serialize route discovery and snapshot replacement. A failed HTTP or model refresh leaves the last loaded snapshot available through `snapshot()`.

Without an explicit scoreboard period, first load the league. Its default follows the Python football `current_week`, even in leagues with multi-week matchups; use `MatchupPeriod` explicitly when needed. Missing sides, opponents, projections and ranks use `Option` where implemented. Player IDs are signed because ESPN uses negative IDs for team defenses. Stat breakdowns retain numeric IDs to avoid collisions between Python labels; `football::stat_label` provides optional known labels.

A runnable example reads optional `ESPN_S2` and `ESPN_SWID` environment variables:

```sh
cargo run --example league -- 123456 2026
```

Append a team ID to print only that team's roster, without requesting the scoreboard:

```sh
cargo run --example league -- 123456 2026 1
```

The team is selected locally from the loaded league snapshot. An unknown team ID returns an error.

The example contacts ESPN; the verification commands below use local fixtures and mock HTTP only. Current-season and successful private-league access have not yet been verified against live ESPN.

## Weekly matchups

Load a league, then request a scoring week. Weeks and matchup periods are separate because some matchups span multiple weeks:

```rust,no_run
use espn_fantasy_football::{Client, LeagueId, ScoringPeriod, Season, TeamId};

# async fn example() -> espn_fantasy_football::Result<()> {
let mut league = Client::builder().build()?.league(LeagueId(394172912), Season(2026))?;
league.fetch().await?;
let weekly = league.box_scores(Some(ScoringPeriod(4))).await?;
if let Some(matchup) = weekly.for_team(TeamId(1)) {
    println!("{matchup:#?}");
}
# Ok(())
# }
```

For a readable table of your lineup and your opponent's lineup:

```sh
cargo run --locked --example weekly -- 394172912 2026 1
cargo run --locked --example weekly -- 394172912 2026 1 4
```

The optional last argument is the scoring week; omitting it uses the loaded current week and matchup period. A future week also uses the current week, matching Python; the example prints the effective week and explains this fallback. Explicit zero weeks and seasons before 2019 are rejected. Each call fetches fresh box scores, a season-level NFL schedule and positional rankings. It does not replace the loaded roster or fetch the entire player pool.

Live team points take precedence over total points. If the live team projection is unavailable, projected player points are summed excluding bench (slot 20) and IR (slot 21). Missing weekly player actual/projected values default to zero for Python compatibility; inspect `entry.player.stats[&weekly.scoring_period]` to distinguish availability. Missing fantasy sides are `None`, so a bye has no fabricated opposing score.

For historical NFL trades, reuse `PlayerTeamHistory::new(season)` with `box_scores_with_history(week, &mut history)` while reading weeks chronologically. Matching-week actual statistics with a nonzero NFL team ID update history; projected-only rows do not. Weeks without actual evidence fall back to history, then the player's current NFL team. A failed request or conversion leaves history unchanged, and histories from another season are rejected.

`BoxScoreContext` accepts an explicit `now_unix_ms` for pure offline conversion. `game_date_unix_ms` retains the kickoff instant; `game_played` reproduces Python's 0/100 kickoff-plus-three-hour heuristic and is not live game progress. NFL schedule parsing uses the first game in the requested period. For compatibility, an opponent is exposed only when the player's default-position ranking map exists; a missing rank is `None`. See [weekly acceptance and ownership](docs/weekly-matchup-contract.md).

## Available players and lookup

Inspect your selected weekly lineup alongside available replacements:

```sh
cargo run --locked --example compare -- 394172912 2026 1
cargo run --locked --example compare -- 394172912 2026 1 current QB 20 0
cargo run --locked --example compare -- 394172912 2026 1 4 WR 20 20
```

Arguments are league, season, team, optional week (`current` to default), eligibility slot, limit and offset. Slot accepts a numeric ID or known label, including QB/0, RB, WR, TE, D/ST and flex slots; `ALL` defaults to all slots. Both groups use the same effective box-score week, including its future-week fallback. The lineup includes bench and IR. Available players retain ESPN's ownership ordering; this example does not rank or recommend moves. Missing point/projection values display as `—`, distinguishing absence from a real zero. These reads do not change the loaded snapshot.

`league.free_agents(FreeAgentOptions::default()).await?` loads one page of up to 50 free agents and waiver players at the loaded current week. Explicit weeks, including future weeks, are passed through. The result exposes `next_offset` only when a full page suggests another page might exist; this is advisory. Pages can change, repeat or be empty. The caller controls pagination and the library never automatically follows pages. Nonzero offsets add a filter extension to Python's default request; explicit zero weeks, zero limits and overflowing offsets fail before weekly requests. Duplicate response identities are errors.

Look up a player ID or exact name:

```sh
cargo run --locked --example lookup -- 394172912 2026 id 3117251
cargo run --locked --example lookup -- 394172912 2026 name "Exact Player Name"
```

`player_by_id` returns `Option<PlayerCard>`; `players_by_ids` and `players_named` return vectors consistently. ID queries remove repeated input IDs and fetch at most 40 per card request, preserving server response order within each batch. Empty input makes no request; ID zero is invalid, while negative defense IDs are supported. Player cards use the loaded final scoring period for stat filters and fetch NFL schedules once after card batches. Missing requested players are omitted; unexpected or duplicate response IDs are rejected. The `raw` card retains transactions and other fields without claiming typed transaction-history support.

Name lookup is exact and case-sensitive and explicitly reloads the active season directory. Unlike Python's first-match behavior, duplicate names resolve to every distinct ID in response order. Unknown names return an empty vector without card or schedule reads. `player_directory()` works before a league load and returns a reusable object for local `ids_named` queries, so callers can choose when to refresh it. Card queries need a loaded league snapshot. There is no implicit directory fetch during construction and no default caching. See [player search contract](docs/player-search-contract.md) for verification scope.

## Verify

Rust 1.85 or newer is required. Run:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo test --locked --doc
```

Checked-in fixtures make the Rust suite independent of Python and of the large untracked reference directory. To additionally exercise original payloads:

```sh
ESPN_PYTHON_REFERENCE=/absolute/path/to/reference cargo test --locked --test parity
```

The optional full-payload tests run only when that variable is supplied. League comparisons cover both 2015 and 2018, all fixture teams and schedule rows, selected roster players/statistics, settings/period metadata, standings and scoreboards. Weekly comparisons cover six synthetic modern cases and one complete historical matchup with all 30 lineup entries. Player comparisons cover the original 2019 card plus synthetic FA/card and directory cases. These are selected semantic comparisons, not assertions of complete package or current-season parity. Add `--test weekly_parity --test player_parity` alongside `--test parity`, or run `--all-targets`, to enable all three full-input comparisons.

To regenerate or verify Python expected results, install `requests` and `requests-mock` in a separate virtual environment and use the frozen reference:

```sh
PYTHONDONTWRITEBYTECODE=1 python scripts/generate_parity_fixtures.py --reference /absolute/path/to/reference --check
PYTHONDONTWRITEBYTECODE=1 python scripts/generate_box_score_fixtures.py --reference /absolute/path/to/reference --check
PYTHONDONTWRITEBYTECODE=1 python scripts/generate_player_fixtures.py --reference /absolute/path/to/reference --check
```

Omit `--check` to regenerate, then review the fixture diff. The generator uses closed request mocks, checks that the full and compact inputs produce identical selected Python outputs, and records source hashes in `tests/fixtures/provenance.json`. It does not modify the reference.

The repository's Rust workflow runs formatting, linting, tests and documentation checks. Python behavior is a compatibility reference; intentional fixes are documented instead of silently reproduced.

## Attribution

Derived football labels, scoring metadata and fixtures come from Christian Wendt's MIT-licensed `espn-api`. The copyright and permission notice are retained in [LICENSE](LICENSE); details are in [NOTICE](NOTICE).
