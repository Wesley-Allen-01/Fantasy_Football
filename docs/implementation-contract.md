# First implementation slice

The current milestone is football league loading, settings, roster/player identity and season stats, ID-linked schedules, standings and remote scoreboard access. Draft enrichment, pro schedules, box scores, free agents, transactions and advanced analytics remain later milestones in the migration plan. No other sports and no ESPN write operations are included.

The coordinator owns Cargo.toml, src/lib.rs, src/ids.rs, src/error.rs, src/client.rs, examples/, README.md and integration tests. Workers share this checkout with disjoint file ownership; do not run repository-wide formatting while another worker is editing. Never edit reference/ or create bytecode/caches within it.

## Shared interfaces

- Crate import: `espn_fantasy_football`.
- IDs are public transparent tuple newtypes in `src/ids.rs`. PlayerId is signed because defenses use negative IDs.
- Error variants and Result are in `src/error.rs`. DTO conversion can wrap serde_json errors as Decode or add response context with InvalidResponse.
- Credentials belongs to the coordinator: owned private strings; `Credentials::new(espn_s2, swid) -> Result<Self>`, `pub(crate) fn cookie_header(&self) -> String`; Debug must redact values.
- Transport worker owns `src/transport.rs` and `tests/transport.rs`. Provide `pub(crate) struct EspnTransport` with `new(http: reqwest::Client, base_url: reqwest::Url, league_id: LeagueId, season: Season, credentials: Option<Credentials>) -> Result<Self>` and `async fn league_get(&mut self, views: &[&str], scoring_period: Option<ScoringPeriod>, filter: Option<&serde_json::Value>, extension: &str) -> Result<serde_json::Value>`. The base URL ends with `/apis/v3/games/`; transport fixes the game to ffl. Extension must be a path suffix, constructed before historical query parameters. HTTP tests can use the public ClientBuilder base_url override once the coordinator supplies it, or private unit tests meanwhile.
- Models worker owns `src/football/` and `tests/models.rs`. Export Player, Team, Settings, Matchup, LeagueSnapshot. Provide `LeagueSnapshot::from_value(value: &serde_json::Value, league_id: LeagueId, season: Season) -> Result<Self>`, public `current_week: ScoringPeriod`, `team(TeamId) -> Option<&Team>`, `standings() -> Vec<&Team>`, and `pub fn scoreboard_from_value(value: &serde_json::Value, period: MatchupPeriod) -> Result<Vec<Matchup>>`. Derive Serialize on domain types for fixture comparison. Agree precise remaining fields directly with the fixture worker; do not edit IDs/errors without asking coordinator.
- Coordinator supplies Client::builder(), builder credentials/timeout/base_url methods and build(), Client::league(LeagueId, Season) -> Result<LeagueHandle>, LeagueHandle::fetch(&mut self) -> Result<LeagueSnapshot>, LeagueHandle::scoreboard(&mut self, Option<MatchupPeriod>) -> Result<Vec<Matchup>>, and LeagueHandle::refresh(&mut self) -> Result<&LeagueSnapshot>. Fetch uses initial views mTeam/mRoster/mMatchup/mSettings/mStandings. Scoreboard uses mMatchupScore and defaults to loaded current_week, matching Python; without a loaded snapshot an explicit period is required. Snapshot replacement occurs only after successful model conversion.
- Fixture worker owns `scripts/`, `tests/fixtures/`, `tests/parity.rs`, and `docs/compatibility-ledger.md`. Create a Python oracle importing the frozen reference and emitting normalized expected data; offline only, no ambient ESPN requests, no source edits. Compare selected 2015/2018 full payload behavior, plus small representative fixtures. Communicate normalization field decisions with models worker. Defer unexplained intentional differences explicitly to the ledger; don't mask differences with broad float tolerance.

## Verification and environment

Run Cargo with `source /workspace/.dev-tools/activate.sh` first (CARGO_HOME/RUSTUP_HOME are writable there). The coordinator controls dependency changes and final cargo fmt/test/clippy. Python investigation venv is `/tmp/espn-investigation-venv`; set `PYTHONDONTWRITEBYTECODE=1`. Dependency network uses configured proxies. Do not contact live ESPN as part of offline checks. Reference content manifest is `/tmp/espn-reference-before.json` and will be rechecked at completion.

Workers should report implemented scope, tests actually run, uncertainties, and required coordinator changes. Finish a small, reviewable implementation rather than extending into later milestones.

## Completed milestone

The transport, model and fixture workers delivered their assigned files; the coordinator integrated the public client and example. An independent review found missing scoreboard-winner metadata and missing/null player stat-source handling that the historical goldens did not cover. Both now have fixes and targeted regressions.

Final local verification on October 2, 2026 passed: 30 Rust tests with the original full-reference parity comparison enabled, one compiled documentation example, formatting, Clippy with warnings denied, and a locked library check on Rust 1.85. The offline Python generator verified all seven checked-in fixture/provenance files. SHA-256 and file-membership comparison confirmed all 189 reference files unchanged. Live current-season and successful private-league validation remain unverified.
