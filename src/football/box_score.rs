//! Weekly lineups and their deterministic, read-only NFL enrichment.
use super::{Player, dto, invalid, league_object, round2, slot_label};
use crate::{
    MatchupPeriod, PlayerId, ProTeamId, Result, ScoringPeriod, Season, SlotId, StatId, TeamId,
};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// Caller-owned evidence from actual weekly statistics, partitioned by season.
/// Calls may be made in any order: actual evidence replaces previous evidence,
/// while a week without actual team evidence leaves the cache unchanged.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PlayerTeamHistory {
    season: Season,
    teams: BTreeMap<PlayerId, ProTeamId>,
}
impl PlayerTeamHistory {
    pub fn new(season: Season) -> Self {
        Self {
            season,
            teams: BTreeMap::new(),
        }
    }
    pub fn season(&self) -> Season {
        self.season
    }
    pub fn teams(&self) -> &BTreeMap<PlayerId, ProTeamId> {
        &self.teams
    }
    pub fn get(&self, player: PlayerId) -> Option<ProTeamId> {
        self.teams.get(&player).copied()
    }
    /// Seed explicit historical knowledge. A team ID of zero is retained, but
    /// ignored when selecting a fallback, matching Python's falsy cache check.
    pub fn insert(&mut self, player: PlayerId, team: ProTeamId) -> Option<ProTeamId> {
        self.teams.insert(player, team)
    }
}

pub struct BoxScoreContext<'a> {
    pub season: Season,
    pub scoring_period: ScoringPeriod,
    pub matchup_period: MatchupPeriod,
    pub now_unix_ms: i64,
    pub history: &'a mut PlayerTeamHistory,
}

#[derive(Clone, Debug, Serialize)]
pub struct WeeklyBoxScores {
    pub season: Season,
    pub scoring_period: ScoringPeriod,
    pub matchup_period: MatchupPeriod,
    pub matchups: Vec<BoxScore>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BoxScore {
    pub id: Option<u64>,
    pub home: Option<BoxTeam>,
    pub away: Option<BoxTeam>,
    pub matchup_type: String,
    pub is_playoff: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct BoxTeam {
    pub team_id: TeamId,
    pub score: f64,
    pub projected: f64,
    pub lineup: Vec<BoxPlayer>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BoxPlayer {
    pub player: Player,
    pub points: f64,
    pub projected_points: f64,
    pub breakdown: BTreeMap<StatId, f64>,
    pub points_breakdown: BTreeMap<StatId, f64>,
    pub projected_breakdown: BTreeMap<StatId, f64>,
    pub projected_points_breakdown: BTreeMap<StatId, f64>,
    pub pro_team: ProTeamId,
    /// Matches Python: only populated when a positional ranking map exists.
    pub pro_opponent: Option<ProTeamId>,
    pub pro_pos_rank: Option<u32>,
    pub game_date_unix_ms: Option<i64>,
    /// Python's heuristic, not live progress: 100 only strictly after kickoff
    /// plus three hours, otherwise 0. A missing schedule defaults to 100.
    pub game_played: u8,
    pub on_bye_week: bool,
    pub slot_position: Option<SlotId>,
}
impl BoxPlayer {
    pub fn slot_position_label(&self) -> Option<&'static str> {
        self.slot_position.map_or(Some("FA"), slot_label)
    }
}

impl WeeklyBoxScores {
    pub fn from_values(
        box_scores: &Value,
        pro_schedule: &Value,
        positional_ratings: &Value,
        context: BoxScoreContext<'_>,
    ) -> Result<Self> {
        if context.history.season != context.season {
            return Err(invalid("player team history belongs to a different season"));
        }
        let data: dto::WeeklySchedule = serde_json::from_value(league_object(box_scores)?.clone())?;
        let pro_schedule = parse_schedule(pro_schedule, context.scoring_period)?;
        let ratings: dto::PositionalRatings = serde_json::from_value(positional_ratings.clone())?;
        // Stage all evidence until every side/player converts successfully.
        let mut history = context.history.clone();
        let mut matchups = Vec::with_capacity(data.schedule.len());
        for matchup in data.schedule {
            let home = matchup
                .home
                .map(|value| {
                    BoxTeam::from_value(&value, &pro_schedule, &ratings, &context, &mut history)
                })
                .transpose()?;
            let away = matchup
                .away
                .map(|value| {
                    BoxTeam::from_value(&value, &pro_schedule, &ratings, &context, &mut history)
                })
                .transpose()?;
            let matchup_type = matchup.playoff_tier_type.unwrap_or_else(|| "NONE".into());
            matchups.push(BoxScore {
                id: matchup.id,
                home,
                away,
                is_playoff: matchup_type != "NONE",
                matchup_type,
            });
        }
        *context.history = history;
        Ok(Self {
            season: context.season,
            scoring_period: context.scoring_period,
            matchup_period: context.matchup_period,
            matchups,
        })
    }
    pub fn for_team(&self, team: TeamId) -> Option<&BoxScore> {
        self.matchups.iter().find(|matchup| {
            matchup
                .home
                .as_ref()
                .is_some_and(|side| side.team_id == team)
                || matchup
                    .away
                    .as_ref()
                    .is_some_and(|side| side.team_id == team)
        })
    }
}

pub(super) type ProGames = BTreeMap<ProTeamId, (ProTeamId, i64)>;
pub(super) fn parse_schedule(value: &Value, week: ScoringPeriod) -> Result<ProGames> {
    let data: dto::ProSchedule = serde_json::from_value(value.clone())?;
    let mut games = BTreeMap::new();
    for team in data.settings.pro_teams {
        if team.id.0 == 0 {
            continue;
        }
        if let Some(value) = team
            .pro_games_by_scoring_period
            .get(&week)
            .and_then(|games| games.first())
        {
            let game: dto::ProGame = serde_json::from_value(value.clone())?;
            let opponent = if team.id == game.away_pro_team_id {
                game.home_pro_team_id
            } else {
                game.away_pro_team_id
            };
            games.insert(team.id, (opponent, game.date));
        }
    }
    Ok(games)
}

impl BoxTeam {
    fn from_value(
        value: &Value,
        schedule: &ProGames,
        ratings: &dto::PositionalRatings,
        context: &BoxScoreContext<'_>,
        history: &mut PlayerTeamHistory,
    ) -> Result<Self> {
        let data: dto::WeeklySide = serde_json::from_value(value.clone())?;
        let live = value
            .as_object()
            .is_some_and(|object| object.contains_key("totalPointsLive"));
        let score = round2(number(
            value,
            if live {
                "totalPointsLive"
            } else {
                "totalPoints"
            },
        )?);
        let mut projected = if live {
            value
                .get("totalProjectedPointsLive")
                .map(|value| serde_json::from_value::<f64>(value.clone()))
                .transpose()?
                .map_or(-1.0, round2)
        } else {
            -1.0
        };
        let mut lineup = Vec::with_capacity(data.roster_for_current_scoring_period.entries.len());
        for value in data.roster_for_current_scoring_period.entries {
            lineup.push(BoxPlayer::from_value(
                &value,
                schedule,
                ratings,
                &PlayerWeekContext::from(context),
                Some(history),
            )?);
        }
        if projected == -1.0 {
            projected = lineup
                .iter()
                .filter(|player| !matches!(player.slot_position, Some(SlotId(20 | 21))))
                .map(|player| player.projected_points)
                .sum();
        }
        Ok(Self {
            team_id: data.team_id,
            score,
            projected,
            lineup,
        })
    }
}
fn number(value: &Value, key: &str) -> Result<f64> {
    let value = value
        .get(key)
        .ok_or_else(|| invalid(&format!("weekly team missing {key}")))?;
    Ok(serde_json::from_value(value.clone())?)
}

pub(super) struct PlayerWeekContext {
    pub season: Season,
    pub scoring_period: ScoringPeriod,
    pub now_unix_ms: i64,
}
impl From<&BoxScoreContext<'_>> for PlayerWeekContext {
    fn from(context: &BoxScoreContext<'_>) -> Self {
        Self {
            season: context.season,
            scoring_period: context.scoring_period,
            now_unix_ms: context.now_unix_ms,
        }
    }
}

impl BoxPlayer {
    pub(super) fn from_value(
        value: &Value,
        schedule: &ProGames,
        ratings: &dto::PositionalRatings,
        context: &PlayerWeekContext,
        history: Option<&mut PlayerTeamHistory>,
    ) -> Result<Self> {
        let canonical = canonical_wrapper(value)?;
        let entry: dto::RosterEntry = serde_json::from_value(canonical.clone())?;
        let metadata: dto::WeeklyPlayerMetadata =
            serde_json::from_value(canonical["playerPoolEntry"]["player"].clone())?;
        let nested = &entry.player_pool_entry.player;
        let default_position = metadata.default_position_id;
        // Match Python: historical team evidence scans matching-week actual
        // records without filtering their season or split type.
        let evidence = metadata
            .stats
            .iter()
            .find(|line| {
                line.scoring_period_id == Some(context.scoring_period)
                    && line.stat_source_id == Some(0)
                    && line.pro_team_id.is_some_and(|team| team.0 != 0)
            })
            .and_then(|line| line.pro_team_id);
        let pro_team = evidence
            .or_else(|| {
                history
                    .as_ref()
                    .and_then(|history| history.get(nested.id))
                    .filter(|team| team.0 != 0)
            })
            .unwrap_or(nested.pro_team_id);
        let mut player = Player::from_dto(entry, context.season)?;
        player.pro_team = pro_team;
        if evidence.is_some() {
            if let Some(history) = history {
                history.insert(player.id, pro_team);
            }
        }
        let mut pro_opponent = None;
        let mut pro_pos_rank = None;
        let mut game_date_unix_ms = None;
        let mut game_played = 100;
        let on_bye_week = !schedule.contains_key(&pro_team);
        if let Some(&(opponent, date)) = schedule.get(&pro_team) {
            game_date_unix_ms = Some(date);
            game_played = if i128::from(context.now_unix_ms) > i128::from(date) + 10_800_000 {
                100
            } else {
                0
            };
            if let Some(position) = default_position.and_then(|position| {
                ratings
                    .position_against_opponent
                    .positional_ratings
                    .get(&position)
            }) {
                pro_opponent = Some(opponent);
                pro_pos_rank = position
                    .ratings_by_opponent
                    .get(&opponent)
                    .map(|rating| rating.rank);
            }
        }
        let period = player.stats.get(&context.scoring_period);
        let actual = period.and_then(|period| period.actual.as_ref());
        let projected = period.and_then(|period| period.projected.as_ref());
        Ok(Self {
            points: actual.map_or(0.0, |line| line.points),
            projected_points: projected.map_or(0.0, |line| line.points),
            breakdown: actual
                .map(|line| line.breakdown.clone())
                .unwrap_or_default(),
            points_breakdown: actual
                .map(|line| line.points_breakdown.clone())
                .unwrap_or_default(),
            projected_breakdown: projected
                .map(|line| line.breakdown.clone())
                .unwrap_or_default(),
            projected_points_breakdown: projected
                .map(|line| line.points_breakdown.clone())
                .unwrap_or_default(),
            slot_position: player.lineup_slot,
            player,
            pro_team,
            pro_opponent,
            pro_pos_rank,
            game_date_unix_ms,
            game_played,
            on_bye_week,
        })
    }
}

/// Support roster-pool and direct-card wrappers through known paths. Identity
/// and eligibility prefer the selected nested player, with explicit top-level
/// fallback for sparse cards; statistics always come from the nested player.
pub(super) fn canonical_wrapper(value: &Value) -> Result<Value> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("weekly player wrapper must be an object"))?;
    let pool = object.get("playerPoolEntry").and_then(Value::as_object);
    let nested = if let Some(pool) = pool {
        pool.get("player")
    } else {
        object.get("player")
    };
    let mut nested = nested
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| invalid("weekly wrapper missing nested player"))?;
    for key in [
        "id",
        "fullName",
        "eligibleSlots",
        "proTeamId",
        "defaultPositionId",
        "positionalRanking",
        "jersey",
    ] {
        if !nested.contains_key(key) {
            if let Some(value) = object.get(key) {
                nested.insert(key.into(), value.clone());
            }
        }
    }
    if !nested.contains_key("positionalRanking") {
        // Player cards expose season positional rank at this known wrapper path.
        // Keep nested/top-level metadata precedence and avoid recursive key search.
        let ranking = pool
            .and_then(|pool| pool.get("ratings"))
            .and_then(|ratings| ratings.get("0"))
            .and_then(|rating| rating.get("positionalRanking"))
            .or_else(|| {
                object
                    .get("ratings")
                    .and_then(|ratings| ratings.get("0"))
                    .and_then(|rating| rating.get("positionalRanking"))
            });
        if let Some(ranking) = ranking {
            nested.insert("positionalRanking".into(), ranking.clone());
        }
    }
    let mut selected_pool = pool.cloned().unwrap_or_default();
    // Direct card IDs carry the same identity contract as pool wrapper IDs.
    // Preserve both IDs so Player::from_dto rejects an inconsistent wrapper.
    if pool.is_none() {
        if let Some(id) = object.get("id") {
            selected_pool.insert("id".into(), id.clone());
        }
    }
    if !selected_pool.contains_key("onTeamId") {
        if let Some(value) = object.get("onTeamId") {
            selected_pool.insert("onTeamId".into(), value.clone());
        }
    }
    selected_pool.insert("player".into(), Value::Object(nested));
    let mut canonical = Map::new();
    for key in [
        "playerId",
        "lineupSlotId",
        "acquisitionType",
        "injuryStatus",
    ] {
        if let Some(value) = object.get(key) {
            canonical.insert(key.into(), value.clone());
        }
    }
    canonical.insert("playerPoolEntry".into(), Value::Object(selected_pool));
    Ok(Value::Object(canonical))
}
