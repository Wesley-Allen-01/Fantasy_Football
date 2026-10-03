//! Typed, read-only access to ESPN fantasy football.
//!
//! Constructing a client or league handle performs no network requests.
//! Fetching returns owned snapshots with ID links instead of cyclic references.
//!
//! ```no_run
//! use espn_fantasy_football::{Client, LeagueId, MatchupPeriod, Season};
//!
//! # async fn example() -> espn_fantasy_football::Result<()> {
//! let client = Client::builder().build()?;
//! let mut league = client.league(LeagueId(123456), Season(2026))?;
//! let snapshot = league.fetch().await?;
//! println!("{} teams", snapshot.teams.len());
//! let matchups = league.scoreboard(Some(MatchupPeriod(1))).await?;
//! println!("{} matchups", matchups.len());
//! # Ok(())
//! # }
//! ```

pub mod client;
pub mod error;
pub mod football;
pub mod ids;
pub(crate) mod player_request;
pub(crate) mod transport;
pub(crate) mod weekly_request;

pub use client::{Client, ClientBuilder, Credentials, LeagueHandle};
pub use error::{Error, Result};
pub use football::{
    BoxPlayer, BoxScore, BoxScoreContext, BoxTeam, PlayerTeamHistory, WeeklyBoxScores,
};
pub use football::{
    DirectoryPlayer, FreeAgentContext, FreeAgentOptions, FreeAgentPage, PlayerCard,
    PlayerDirectory, PlayerGame,
};
pub use football::{LeagueSnapshot, Matchup, Player, Settings, Team};
pub use ids::{
    LeagueId, MatchupPeriod, PlayerId, ProTeamId, ScoringPeriod, Season, SlotId, StatId, TeamId,
};
