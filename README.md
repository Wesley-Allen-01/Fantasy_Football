# ESPN Fantasy Football in Rust

An asynchronous, read-only Rust migration of the football API in `reference/`, based on Python `espn-api` 0.46.0. The reference is kept unchanged. League foundation and weekly matchups are implemented; complete football package parity remains in progress.

## Implemented

- Reusable HTTP client, paired private-league cookies, historical league routes and stateful endpoint fallback.
- Explicit league loading and atomic refresh, without network requests during construction.
- Football settings and scoring metadata, teams/members/owners, rosters and actual/projected season statistics.
- ID-linked schedules, missing-side/bye handling, team lookup and ESPN standings.
- Remote scoreboards with the Python request filters and default period behavior.
- Weekly box scores and both lineups, live score precedence, starter projection fallback, NFL schedules and opponent positional rankings.
- Explicit season-scoped historical NFL team evidence for traded players, with atomic updates after a complete successful parse.
- Offline Python-derived comparison fixtures and an optional comparison against full original payloads.

Free agents, player-card lookup, historical roster loading, draft/history/reporting, weekly standings and power rankings are later milestones. Submitting lineups, claims or trades is outside the reference's read-only API scope. See the [migration plan](docs/espn-rust-implementation-plan.md) and [compatibility ledger](docs/compatibility-ledger.md) for preserved behaviors, intentional corrections and evidence gaps.

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

The optional full-payload tests run only when that variable is supplied. League comparisons cover both 2015 and 2018, all fixture teams and schedule rows, selected roster players/statistics, settings/period metadata, standings and scoreboards. Weekly comparisons cover six synthetic modern cases and one complete historical matchup with all 30 lineup entries. These are selected semantic comparisons, not assertions of complete package or current-season parity. Include `--test weekly_parity` alongside `--test parity` to enable both full-input comparisons.

To regenerate or verify Python expected results, install `requests` and `requests-mock` in a separate virtual environment and use the frozen reference:

```sh
PYTHONDONTWRITEBYTECODE=1 python scripts/generate_parity_fixtures.py --reference /absolute/path/to/reference --check
PYTHONDONTWRITEBYTECODE=1 python scripts/generate_box_score_fixtures.py --reference /absolute/path/to/reference --check
```

Omit `--check` to regenerate, then review the fixture diff. The generator uses closed request mocks, checks that the full and compact inputs produce identical selected Python outputs, and records source hashes in `tests/fixtures/provenance.json`. It does not modify the reference.

The repository's Rust workflow runs formatting, linting, tests and documentation checks. Python behavior is a compatibility reference; intentional fixes are documented instead of silently reproduced.

## Attribution

Derived football labels, scoring metadata and fixtures come from Christian Wendt's MIT-licensed `espn-api`. The copyright and permission notice are retained in [LICENSE](LICENSE); details are in [NOTICE](NOTICE).
