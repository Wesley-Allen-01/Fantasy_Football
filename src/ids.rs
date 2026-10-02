//! ESPN identifiers are kept separate even when they share a numeric encoding.
use serde::{Deserialize, Serialize};
use std::fmt;

macro_rules! id {
    ($name:ident, $value:ty) => {
        #[derive(
            Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, Hash,
        )]
        #[serde(transparent)]
        pub struct $name(pub $value);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id!(LeagueId, u64);
id!(Season, u16);
id!(TeamId, u32);
// ESPN uses negative IDs for team defenses.
id!(PlayerId, i64);
id!(ProTeamId, u32);
id!(ScoringPeriod, u32);
id!(MatchupPeriod, u32);
id!(SlotId, u32);
id!(StatId, u32);
