//! Owned football snapshots with ID links and explicit missing values.
//!
//! ESPN's numeric slot/stat IDs remain authoritative. Labels are optional and
//! unknown IDs survive decoding. Conversion performs no I/O or enrichment.
mod box_score;
mod dto;
pub use box_score::{
    BoxPlayer, BoxScore, BoxScoreContext, BoxTeam, PlayerTeamHistory, WeeklyBoxScores,
};
mod labels;
mod players;
pub use labels::{pro_team_abbreviation, slot_label, stat_label};
pub use players::{
    DirectoryPlayer, FreeAgentContext, FreeAgentOptions, FreeAgentPage, PlayerCard,
    PlayerDirectory, PlayerGame,
};

use crate::{
    Error, LeagueId, MatchupPeriod, PlayerId, ProTeamId, Result, ScoringPeriod, Season, SlotId,
    StatId, TeamId,
};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize)]
pub struct LeagueSnapshot {
    pub league_id: LeagueId,
    pub season: Season,
    /// Python's current_week: capped at final_scoring_period from 2018 onward.
    pub current_week: ScoringPeriod,
    /// Raw NFL latest scoring period; independent of the capped league week.
    /// Missing upstream metadata is tolerated explicitly as None.
    pub nfl_week: Option<ScoringPeriod>,
    pub scoring_period: ScoringPeriod,
    pub current_matchup_period: MatchupPeriod,
    pub first_scoring_period: ScoringPeriod,
    pub final_scoring_period: ScoringPeriod,
    pub previous_seasons: Vec<Season>,
    pub settings: Settings,
    /// Full member records, also linked to team owners by member ID.
    pub members: Vec<Value>,
    /// Sorted by team ID; roster order and schedule response order are retained.
    pub teams: Vec<Team>,
    pub schedule: Vec<Matchup>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Settings {
    pub name: String,
    pub team_count: u32,
    pub reg_season_count: u32,
    pub matchup_periods: BTreeMap<MatchupPeriod, Vec<ScoringPeriod>>,
    pub playoff_team_count: u32,
    pub playoff_matchup_period_length: u32,
    pub keeper_count: u32,
    pub veto_votes_required: u32,
    /// ESPN epoch milliseconds; absence is explicit rather than a zero date.
    pub trade_deadline: Option<i64>,
    pub trade_revision_hours: Option<f64>,
    pub tie_rule: String,
    pub playoff_tie_rule: String,
    pub playoff_seed_tie_rule: String,
    pub scoring_type: Option<String>,
    pub median_scoring: bool,
    pub faab: bool,
    pub acquisition_budget: f64,
    pub acquisition_limit: Option<f64>,
    pub matchup_acquisition_limit: Option<f64>,
    pub matchup_limit_per_scoring_period: Option<bool>,
    pub minimum_bid: f64,
    pub waiver_process_days: Vec<String>,
    pub waiver_process_hour: Option<u32>,
    pub divisions: BTreeMap<u32, Option<String>>,
    /// Keyed by the actual slot ID, independent of JSON object ordering.
    pub position_slot_counts: BTreeMap<SlotId, u32>,
    pub scoring_format: Vec<ScoringRule>,
    /// Retain unknown settings for future feature work and consumers.
    pub raw_scoring_settings: Value,
    pub raw_schedule_settings: Value,
    pub raw: Value,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScoringRule {
    pub id: StatId,
    pub abbreviation: String,
    pub label: String,
    /// Effective value: an explicit D/ST override, including zero, wins.
    pub points: f64,
    pub base_points: f64,
    /// Preserve all slot overrides independently of the effective shorthand.
    pub points_overrides: BTreeMap<SlotId, f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    pub abbreviation: String,
    pub division_id: u32,
    pub division_name: String,
    pub wins: u32,
    pub losses: u32,
    pub ties: u32,
    pub points_for: f64,
    pub points_against: f64,
    pub acquisitions: u32,
    pub acquisition_budget_spent: f64,
    pub drops: u32,
    pub trades: u32,
    pub move_to_ir: u32,
    pub playoff_pct: f64,
    pub draft_projected_rank: u32,
    pub streak_length: u32,
    pub streak_type: String,
    pub standing: u32,
    pub final_standing: Option<u32>,
    pub waiver_rank: u32,
    pub logo_url: String,
    pub owner_ids: Vec<String>,
    pub owners: Vec<Value>,
    pub roster: Vec<Player>,
    pub schedule: Vec<TeamMatchup>,
    pub stats: BTreeMap<StatId, f64>,
}

impl Team {
    pub fn player(&self, id: PlayerId) -> Option<&Player> {
        self.roster.iter().find(|player| player.id == id)
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Player {
    pub id: PlayerId,
    pub name: String,
    pub positional_rank: Option<u32>,
    pub eligible_slots: Vec<SlotId>,
    pub acquisition_type: Option<String>,
    pub pro_team: ProTeamId,
    /// String and numeric jersey representations are normalized to text.
    pub jersey: Option<String>,
    pub injury_status: Option<String>,
    pub injured: bool,
    pub on_team_id: Option<TeamId>,
    pub lineup_slot: Option<SlotId>,
    /// First eligible non-rookie, non-flex slot, following Python's rule.
    pub position: Option<SlotId>,
    pub percent_owned: f64,
    pub percent_started: f64,
    pub active_status: ActiveStatus,
    pub stats: BTreeMap<ScoringPeriod, PeriodStats>,
    pub total_points: f64,
    pub projected_total_points: f64,
    pub avg_points: f64,
    pub projected_avg_points: f64,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ActiveStatus {
    Bye,
    Inactive,
    Active,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct PeriodStats {
    pub actual: Option<StatLine>,
    pub projected: Option<StatLine>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatLine {
    pub points: f64,
    pub average_points: f64,
    /// Raw game statistics keyed by IDs; aliases cannot silently overwrite values.
    pub breakdown: BTreeMap<StatId, f64>,
    pub points_breakdown: BTreeMap<StatId, f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Matchup {
    pub id: Option<u64>,
    pub period: MatchupPeriod,
    pub home_team: Option<TeamId>,
    pub away_team: Option<TeamId>,
    pub home_score: Option<f64>,
    pub away_score: Option<f64>,
    /// Additional ESPN metadata, including future unknown tokens. League schedules
    /// require it for outcomes; scoreboard entries may omit it. No winner is inferred.
    pub winner: Option<String>,
    pub matchup_type: String,
    pub is_playoff: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct TeamMatchup {
    pub matchup_id: Option<u64>,
    pub period: MatchupPeriod,
    /// None represents a bye, rather than a cyclic link back to this team.
    pub opponent: Option<TeamId>,
    pub score: Option<f64>,
    pub outcome: Outcome,
    /// Computed from the two sides of this matchup, not vector indices.
    pub margin: Option<f64>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
pub enum Outcome {
    #[serde(rename = "U")]
    Undecided,
    #[serde(rename = "T")]
    Tie,
    #[serde(rename = "W")]
    Win,
    #[serde(rename = "L")]
    Loss,
}

impl LeagueSnapshot {
    pub fn from_value(value: &Value, league_id: LeagueId, season: Season) -> Result<Self> {
        let value = league_object(value)?;
        let data: dto::League = serde_json::from_value(value.clone())?;
        if data.id.is_some_and(|id| id != league_id) || data.season_id != season {
            return Err(invalid(
                "league or season ID differs from the requested league",
            ));
        }
        let settings = Settings::from_dto(data.settings, &value["settings"]);
        let schedule: Vec<Matchup> = data.schedule.into_iter().map(Matchup::from).collect();
        let mut ids = BTreeSet::new();
        let mut teams = Vec::with_capacity(data.teams.len());
        for team in data.teams {
            if !ids.insert(team.id) {
                return Err(invalid("duplicate team ID"));
            }
            teams.push(Team::from_dto(
                team,
                &data.members,
                &settings,
                &schedule,
                season,
            )?);
        }
        teams.sort_by_key(|team| team.id);
        let current_week = if season.0 < 2018 {
            data.scoring_period_id
        } else {
            data.scoring_period_id.min(data.status.final_scoring_period)
        };
        Ok(Self {
            league_id,
            season,
            current_week,
            nfl_week: data.status.latest_scoring_period,
            scoring_period: data.scoring_period_id,
            current_matchup_period: data.status.current_matchup_period,
            first_scoring_period: data.status.first_scoring_period,
            final_scoring_period: data.status.final_scoring_period,
            previous_seasons: data
                .status
                .previous_seasons
                .into_iter()
                .filter(|year| *year < season)
                .collect(),
            settings,
            members: data.members,
            teams,
            schedule,
        })
    }

    pub fn team(&self, id: TeamId) -> Option<&Team> {
        self.teams.iter().find(|team| team.id == id)
    }

    /// ESPN's final ranking if nonzero, otherwise the current playoff seed.
    /// Ties retain team-ID ordering. No weekly tiebreaker simulation is performed.
    pub fn standings(&self) -> Vec<&Team> {
        let mut teams: Vec<_> = self.teams.iter().collect();
        teams.sort_by_key(|team| {
            team.final_standing
                .filter(|rank| *rank != 0)
                .unwrap_or(team.standing)
        });
        teams
    }
}

/// Filter by matchup period (not scoring period), preserving ESPN response order.
pub fn scoreboard_from_value(value: &Value, period: MatchupPeriod) -> Result<Vec<Matchup>> {
    let data: dto::Scoreboard = serde_json::from_value(league_object(value)?.clone())?;
    let mut matchups = Vec::new();
    for value in data.schedule {
        let selection: dto::ScoreboardPeriod = serde_json::from_value(value.clone())?;
        if selection.matchup_period_id == period {
            let entry: dto::ScoreboardMatchup = serde_json::from_value(value)?;
            matchups.push(Matchup::from(entry));
        }
    }
    Ok(matchups)
}

fn league_object(value: &Value) -> Result<&Value> {
    match value {
        Value::Object(_) => Ok(value),
        Value::Array(entries) if entries.len() == 1 && entries[0].is_object() => Ok(&entries[0]),
        _ => Err(invalid(
            "expected a league object or a historical singleton league array",
        )),
    }
}
fn invalid(context: &str) -> Error {
    Error::InvalidResponse {
        context: context.to_owned(),
    }
}

impl Settings {
    fn from_dto(data: dto::Settings, raw: &Value) -> Self {
        let schedule = data.schedule_settings;
        let scoring = data.scoring_settings;
        let acquisition = data.acquisition_settings;
        Self {
            name: data.name,
            team_count: data.size,
            reg_season_count: schedule.matchup_period_count,
            matchup_periods: schedule.matchup_periods,
            playoff_team_count: schedule.playoff_team_count,
            playoff_matchup_period_length: schedule.playoff_matchup_period_length,
            keeper_count: data.draft_settings.keeper_count,
            veto_votes_required: data.trade_settings.veto_votes_required,
            trade_deadline: data.trade_settings.deadline_date,
            trade_revision_hours: data.trade_settings.revision_hours,
            tie_rule: scoring.matchup_tie_rule,
            playoff_tie_rule: scoring.playoff_matchup_tie_rule,
            playoff_seed_tie_rule: schedule.playoff_seeding_rule,
            scoring_type: scoring.scoring_type,
            median_scoring: scoring.scoring_enhancement_type.as_deref()
                == Some("WIN_BONUS_TOP_HALF"),
            faab: acquisition.is_using_acquisition_budget,
            acquisition_budget: acquisition.acquisition_budget,
            acquisition_limit: acquisition.acquisition_limit,
            matchup_acquisition_limit: acquisition.matchup_acquisition_limit,
            matchup_limit_per_scoring_period: acquisition.matchup_limit_per_scoring_period,
            minimum_bid: acquisition.minimum_bid,
            waiver_process_days: acquisition.waiver_process_days,
            waiver_process_hour: acquisition.waiver_process_hour,
            divisions: schedule
                .divisions
                .into_iter()
                .map(|division| (division.id, division.name))
                .collect(),
            position_slot_counts: data.roster_settings.lineup_slot_counts,
            scoring_format: scoring
                .scoring_items
                .into_iter()
                .map(|item| {
                    let (abbreviation, label) = labels::scoring_label(item.stat_id);
                    let points = item
                        .points_overrides
                        .get(&SlotId(16))
                        .copied()
                        .unwrap_or(item.points);
                    ScoringRule {
                        id: item.stat_id,
                        abbreviation: abbreviation.into(),
                        label: label.into(),
                        points,
                        base_points: item.points,
                        points_overrides: item.points_overrides,
                    }
                })
                .collect(),
            raw_scoring_settings: raw["scoringSettings"].clone(),
            raw_schedule_settings: raw["scheduleSettings"].clone(),
            raw: raw.clone(),
        }
    }
}

impl Team {
    fn from_dto(
        data: dto::Team,
        members: &[Value],
        settings: &Settings,
        schedule: &[Matchup],
        season: Season,
    ) -> Result<Self> {
        let mut roster = Vec::with_capacity(data.roster.entries.len());
        for entry in data.roster.entries {
            roster.push(Player::from_dto(entry, season)?);
        }
        let owners = members
            .iter()
            .filter(|member| {
                member
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| data.owners.iter().any(|owner| owner == id))
            })
            .cloned()
            .collect();
        let name = match data.name.as_deref() {
            Some(name) if name != "Unknown" => name.to_owned(),
            _ => format!(
                "{} {}",
                data.location.as_deref().unwrap_or("Unknown"),
                data.nickname.as_deref().unwrap_or("Unknown")
            ),
        };
        let team_schedule = schedule
            .iter()
            .filter_map(|matchup| matchup.for_team(data.id))
            .collect();
        Ok(Self {
            id: data.id,
            name,
            abbreviation: data.abbrev,
            division_id: data.division_id,
            division_name: settings
                .divisions
                .get(&data.division_id)
                .and_then(|name| name.clone())
                .unwrap_or_default(),
            wins: data.record.overall.wins,
            losses: data.record.overall.losses,
            ties: data.record.overall.ties,
            points_for: data.record.overall.points_for,
            points_against: round2(data.record.overall.points_against),
            acquisitions: data.transaction_counter.acquisitions,
            acquisition_budget_spent: data.transaction_counter.acquisition_budget_spent,
            drops: data.transaction_counter.drops,
            trades: data.transaction_counter.trades,
            move_to_ir: data.transaction_counter.move_to_ir,
            playoff_pct: data.current_simulation_results.playoff_pct * 100.0,
            draft_projected_rank: data.draft_day_projected_rank,
            streak_length: data.record.overall.streak_length,
            streak_type: data.record.overall.streak_type,
            standing: data.playoff_seed,
            final_standing: data
                .rank_final
                .filter(|rank| *rank != 0)
                .or(data.rank_calculated_final),
            waiver_rank: data.waiver_rank,
            logo_url: data.logo.unwrap_or_default(),
            owner_ids: data.owners,
            owners,
            roster,
            schedule: team_schedule,
            stats: data.values_by_stat,
        })
    }
}

impl Player {
    fn from_dto(entry: dto::RosterEntry, season: Season) -> Result<Self> {
        let pool = entry.player_pool_entry;
        let player = pool.player;
        if entry.player_id.is_some_and(|id| id != player.id)
            || pool.id.is_some_and(|id| id != player.id)
        {
            return Err(invalid("roster player ID differs from nested player ID"));
        }
        let mut stats: BTreeMap<ScoringPeriod, PeriodStats> = BTreeMap::new();
        let mut active_status = ActiveStatus::Bye;
        for line in player.stats {
            if line.season_id != Some(season) || line.stat_split_type_id == Some(2) {
                continue;
            }
            let stat = StatLine {
                points: round2(line.applied_total),
                average_points: round2(line.applied_average),
                breakdown: line.stats,
                points_breakdown: line.applied_stats,
            };
            let period = stats.entry(line.scoring_period_id).or_default();
            // Preserve Python's falsy-source quirk: absent/null source updates
            // active status, but its points still belong to projected statistics.
            if line.stat_source_id.is_none_or(|source| source == 0) {
                active_status = if stat.breakdown.is_empty() {
                    ActiveStatus::Inactive
                } else {
                    ActiveStatus::Active
                };
            }
            if line.stat_source_id == Some(0) {
                period.actual = Some(stat);
            } else {
                period.projected = Some(stat);
            }
        }
        let total = stats.get(&ScoringPeriod(0));
        let actual = total.and_then(|period| period.actual.as_ref());
        let projected = total.and_then(|period| period.projected.as_ref());
        let total_points = actual.map_or(0.0, |line| line.points);
        let projected_total_points = projected.map_or(0.0, |line| line.points);
        let avg_points = actual.map_or(0.0, |line| line.average_points);
        let projected_avg_points = projected.map_or(0.0, |line| line.average_points);
        let position = player.eligible_slots.iter().copied().find(|slot| {
            player.full_name.contains('/')
                || (slot.0 != 25 && slot_label(*slot).is_some_and(|label| !label.contains('/')))
        });
        let jersey = player.jersey.and_then(|jersey| match jersey {
            Value::String(text) => Some(text),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        });
        Ok(Self {
            id: player.id,
            name: player.full_name,
            positional_rank: player.positional_ranking,
            eligible_slots: player.eligible_slots,
            acquisition_type: entry.acquisition_type,
            pro_team: player.pro_team_id,
            jersey,
            injury_status: player.injury_status.or(entry.injury_status),
            injured: player.injured,
            on_team_id: pool.on_team_id,
            lineup_slot: entry.lineup_slot_id,
            position,
            percent_owned: round2(player.ownership.percent_owned.unwrap_or(-1.0)),
            percent_started: round2(player.ownership.percent_started.unwrap_or(-1.0)),
            active_status,
            stats,
            total_points,
            projected_total_points,
            avg_points,
            projected_avg_points,
        })
    }
}

impl From<dto::Matchup> for Matchup {
    fn from(data: dto::Matchup) -> Self {
        let matchup_type = data.playoff_tier_type.unwrap_or_else(|| "NONE".into());
        Self {
            id: data.id,
            period: data.matchup_period_id,
            home_team: data.home.as_ref().map(|side| side.team_id),
            away_team: data.away.as_ref().map(|side| side.team_id),
            home_score: data.home.and_then(|side| side.total_points),
            away_score: data.away.and_then(|side| side.total_points),
            winner: Some(data.winner),
            is_playoff: matchup_type != "NONE",
            matchup_type,
        }
    }
}

impl From<dto::ScoreboardMatchup> for Matchup {
    fn from(data: dto::ScoreboardMatchup) -> Self {
        let matchup_type = data.playoff_tier_type.unwrap_or_else(|| "NONE".into());
        Self {
            id: data.id,
            period: data.matchup_period_id,
            home_team: data.home.as_ref().map(|side| side.team_id),
            away_team: data.away.as_ref().map(|side| side.team_id),
            home_score: data.home.map(|side| side.total_points),
            away_score: data.away.map(|side| side.total_points),
            winner: data.winner,
            is_playoff: matchup_type != "NONE",
            matchup_type,
        }
    }
}

impl Matchup {
    fn for_team(&self, id: TeamId) -> Option<TeamMatchup> {
        let (opponent, score, opposing_score, away) = if self.home_team == Some(id) {
            (self.away_team, self.home_score, self.away_score, false)
        } else if self.away_team == Some(id) {
            (self.home_team, self.away_score, self.home_score, true)
        } else {
            return None;
        };
        let outcome = match self.winner.as_deref() {
            Some("UNDECIDED") | None => Outcome::Undecided,
            Some("TIE") => Outcome::Tie,
            Some("AWAY") if away => Outcome::Win,
            Some("HOME") if !away => Outcome::Win,
            _ => Outcome::Loss,
        };
        Some(TeamMatchup {
            matchup_id: self.id,
            period: self.period,
            opponent,
            score,
            outcome,
            margin: score
                .zip(opposing_score)
                .map(|(score, other)| score - other),
        })
    }
}

/// Decimal formatting rounds the actual binary value with ties to even, like
/// Python round(value, 2). Multiplying by 100 first would change edge cases.
fn round2(value: f64) -> f64 {
    format!("{value:.2}")
        .parse()
        .expect("formatted f64 is a valid number")
}
