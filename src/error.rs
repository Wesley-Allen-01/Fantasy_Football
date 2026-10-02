use crate::LeagueId;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid client configuration: {0}")]
    Configuration(String),
    #[error("ESPN request failed: {0}")]
    Network(#[from] reqwest::Error),
    #[error("ESPN returned HTTP {status}")]
    Http { status: u16 },
    #[error("league {league_id} does not exist")]
    InvalidLeague { league_id: LeagueId },
    #[error("ESPN access denied (missing credential pair: {missing_credentials})")]
    AccessDenied { missing_credentials: bool },
    #[error("invalid ESPN JSON: {0}")]
    Decode(#[from] serde_json::Error),
    #[error("invalid ESPN response: {context}")]
    InvalidResponse { context: String },
}
