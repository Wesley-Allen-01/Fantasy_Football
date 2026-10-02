# ESPN Fantasy Football in Rust

An asynchronous, read-only Rust migration of the football API in `reference/`, based on Python `espn-api` 0.46.0. The reference is kept unchanged. This is the first working slice, not yet complete football package parity.

## Implemented

- Reusable HTTP client, paired private-league cookies, historical league routes and stateful endpoint fallback.
- Explicit league loading and atomic refresh, without network requests during construction.
- Football settings and scoring metadata, teams/members/owners, rosters and actual/projected season statistics.
- ID-linked schedules, missing-side/bye handling, team lookup and ESPN standings.
- Remote scoreboards with the Python request filters and default period behavior.
- Offline Python-derived comparison fixtures and an optional comparison against full original payloads.

Weekly box scores, free agents, player-card lookup, draft/history/reporting, weekly standings and power rankings are later milestones. Submitting lineups, claims or trades is outside the reference's read-only API scope. See the [migration plan](docs/espn-rust-implementation-plan.md) and [compatibility ledger](docs/compatibility-ledger.md) for preserved behaviors, intentional corrections and evidence gaps.

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

The example contacts ESPN; the verification commands below use local fixtures and mock HTTP only. Current-season and successful private-league access have not yet been verified against live ESPN.

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

The optional full-payload test runs only when that variable is supplied. Comparisons cover both 2015 and 2018, all fixture teams and schedule rows, selected roster players/statistics, settings/period metadata, standings and scoreboards. They are selected semantic comparisons, not assertions of complete package or current-season parity.

To regenerate or verify Python expected results, install `requests` and `requests-mock` in a separate virtual environment and use the frozen reference:

```sh
PYTHONDONTWRITEBYTECODE=1 python scripts/generate_parity_fixtures.py --reference /absolute/path/to/reference --check
```

Omit `--check` to regenerate, then review the fixture diff. The generator uses closed request mocks, checks that the full and compact inputs produce identical selected Python outputs, and records source hashes in `tests/fixtures/provenance.json`. It does not modify the reference.

The repository's Rust workflow runs formatting, linting, tests and documentation checks. Python behavior is a compatibility reference; intentional fixes are documented instead of silently reproduced.

## Attribution

Derived football labels, scoring metadata and fixtures come from Christian Wendt's MIT-licensed `espn-api`. The copyright and permission notice are retained in [LICENSE](LICENSE); details are in [NOTICE](NOTICE).
