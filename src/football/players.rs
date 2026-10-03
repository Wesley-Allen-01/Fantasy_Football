//! Available-player pages, detailed cards, and exact-name identity lookup.
use super::{
    BoxPlayer, Player,
    box_score::{PlayerWeekContext, canonical_wrapper, parse_schedule},
    dto, invalid,
};
use crate::{PlayerId, ProTeamId, Result, ScoringPeriod, Season, SlotId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct FreeAgentOptions {
    pub week: Option<ScoringPeriod>,
    pub limit: u32,
    pub offset: u32,
    pub slots: Vec<SlotId>,
}
impl Default for FreeAgentOptions {
    fn default() -> Self {
        Self {
            week: None,
            limit: 50,
            offset: 0,
            slots: Vec::new(),
        }
    }
}

pub struct FreeAgentContext {
    pub season: Season,
    pub scoring_period: ScoringPeriod,
    pub now_unix_ms: i64,
    pub offset: u32,
    pub limit: u32,
}
#[derive(Clone, Debug, Serialize)]
pub struct FreeAgentPage {
    pub season: Season,
    pub scoring_period: ScoringPeriod,
    pub offset: u32,
    pub limit: u32,
    /// ESPN response order; no projection sorting or recommendation is applied.
    pub players: Vec<BoxPlayer>,
    /// Advisory only: a full page does not establish that another page exists.
    pub next_offset: Option<u32>,
}
impl FreeAgentPage {
    pub fn from_values(
        players: &Value,
        pro_schedule: &Value,
        ratings: &Value,
        context: FreeAgentContext,
    ) -> Result<Self> {
        if context.limit == 0 {
            return Err(invalid("free-agent limit must be nonzero"));
        }
        context
            .offset
            .checked_add(context.limit)
            .ok_or_else(|| invalid("free-agent page offset overflows"))?;
        let data: PlayerList = serde_json::from_value(players.clone())?;
        let schedule = parse_schedule(pro_schedule, context.scoring_period)?;
        let ratings: dto::PositionalRatings = serde_json::from_value(ratings.clone())?;
        let context_week = PlayerWeekContext {
            season: context.season,
            scoring_period: context.scoring_period,
            now_unix_ms: context.now_unix_ms,
        };
        let mut ids = BTreeSet::new();
        let mut players = Vec::with_capacity(data.players.len());
        for value in data.players {
            let player = BoxPlayer::from_value(&value, &schedule, &ratings, &context_week, None)?;
            if player.player.id.0 == 0 {
                return Err(invalid("free-agent player ID must be nonzero"));
            }
            if !ids.insert(player.player.id) {
                return Err(invalid("duplicate free-agent player ID"));
            }
            players.push(player);
        }
        let count = u32::try_from(players.len())
            .map_err(|_| invalid("free-agent result count overflows"))?;
        let next_offset = if count >= context.limit {
            Some(
                context
                    .offset
                    .checked_add(count)
                    .ok_or_else(|| invalid("free-agent next offset overflows"))?,
            )
        } else {
            None
        };
        Ok(Self {
            season: context.season,
            scoring_period: context.scoring_period,
            offset: context.offset,
            limit: context.limit,
            players,
            next_offset,
        })
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PlayerCard {
    pub player: Player,
    /// First scheduled game in each period for the player's current NFL team.
    pub schedule: BTreeMap<ScoringPeriod, PlayerGame>,
    /// Preserve transactions/acquisition and unknown fields without claiming
    /// typed transaction history behavior in this milestone.
    pub raw: Value,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PlayerGame {
    pub opponent: ProTeamId,
    pub date_unix_ms: i64,
}
impl PlayerCard {
    pub fn from_values(cards: &Value, pro_schedule: &Value, season: Season) -> Result<Vec<Self>> {
        let data: PlayerList = serde_json::from_value(cards.clone())?;
        let schedule = card_pro_teams(pro_schedule)?;
        let mut ids = BTreeSet::new();
        let mut cards = Vec::with_capacity(data.players.len());
        for raw in data.players {
            // Card parsing deliberately does not deserialize weekly team evidence
            // or positional metadata, which are unused in a full-season card.
            let entry: dto::RosterEntry = serde_json::from_value(canonical_wrapper(&raw)?)?;
            let player = Player::from_dto(entry, season)?;
            if player.id.0 == 0 {
                return Err(invalid("card player ID must be nonzero"));
            }
            if !ids.insert(player.id) {
                return Err(invalid("duplicate card player ID"));
            }
            let mut games = BTreeMap::new();
            // Match Python's map construction: later records for a repeated
            // pro-team ID take precedence over earlier records.
            if let Some(team) = schedule
                .iter()
                .rev()
                .find(|team| team.id == player.pro_team)
            {
                for (&period, entries) in &team.pro_games_by_scoring_period {
                    if let Some(value) = entries.first() {
                        let game: dto::ProGame = serde_json::from_value(value.clone())?;
                        let opponent = if game.away_pro_team_id != player.pro_team {
                            game.away_pro_team_id
                        } else {
                            game.home_pro_team_id
                        };
                        games.insert(
                            period,
                            PlayerGame {
                                opponent,
                                date_unix_ms: game.date,
                            },
                        );
                    }
                }
            }
            cards.push(Self {
                player,
                schedule: games,
                raw,
            });
        }
        Ok(cards)
    }
}

/// Card reads preserve Python's _get_all_pro_schedule defaults. Missing object
/// keys and an empty proTeams object mean no schedule; malformed/null values
/// remain errors. Weekly schedule reads deliberately keep their stricter DTO.
fn card_pro_teams(value: &Value) -> Result<Vec<dto::ProTeamSchedule>> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("card pro schedule must be an object"))?;
    let Some(settings) = object.get("settings") else {
        return Ok(Vec::new());
    };
    let settings = settings
        .as_object()
        .ok_or_else(|| invalid("card pro schedule settings must be an object"))?;
    let Some(teams) = settings.get("proTeams") else {
        return Ok(Vec::new());
    };
    match teams {
        Value::Array(_) => Ok(serde_json::from_value(teams.clone())?),
        Value::Object(teams) if teams.is_empty() => Ok(Vec::new()),
        _ => Err(invalid("card proTeams must be an array or an empty object")),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PlayerDirectory {
    pub season: Season,
    pub players: Vec<DirectoryPlayer>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DirectoryPlayer {
    pub id: PlayerId,
    pub name: String,
}
impl PlayerDirectory {
    pub fn from_value(value: &Value, season: Season) -> Result<Self> {
        let data: Vec<DirectoryEntry> = serde_json::from_value(value.clone())?;
        let mut seen: BTreeMap<PlayerId, String> = BTreeMap::new();
        let mut players = Vec::with_capacity(data.len());
        for entry in data {
            if entry.id.0 == 0 || entry.full_name.trim().is_empty() {
                return Err(invalid(
                    "directory player identity must have a nonzero ID and nonblank name",
                ));
            }
            if let Some(name) = seen.get(&entry.id) {
                if name != &entry.full_name {
                    return Err(invalid("directory player ID has conflicting names"));
                }
                continue;
            }
            seen.insert(entry.id, entry.full_name.clone());
            players.push(DirectoryPlayer {
                id: entry.id,
                name: entry.full_name,
            });
        }
        Ok(Self { season, players })
    }
    /// Exact, case-sensitive matching; retain distinct IDs in ESPN response order.
    pub fn ids_named(&self, name: &str) -> Vec<PlayerId> {
        self.players
            .iter()
            .filter(|player| player.name == name)
            .map(|player| player.id)
            .collect()
    }
}
#[derive(Deserialize)]
struct PlayerList {
    players: Vec<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryEntry {
    id: PlayerId,
    full_name: String,
}
