//! Private wire types: decode known paths rather than recursively finding keys.
use crate::{
    LeagueId, MatchupPeriod, PlayerId, ProTeamId, ScoringPeriod, Season, SlotId, StatId, TeamId,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct League {
    pub id: Option<LeagueId>,
    pub season_id: Season,
    pub scoring_period_id: ScoringPeriod,
    pub status: Status,
    pub settings: Settings,
    #[serde(default)]
    pub members: Vec<Value>,
    pub teams: Vec<Team>,
    pub schedule: Vec<Matchup>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Status {
    pub current_matchup_period: MatchupPeriod,
    pub latest_scoring_period: Option<ScoringPeriod>,
    pub first_scoring_period: ScoringPeriod,
    pub final_scoring_period: ScoringPeriod,
    #[serde(default)]
    pub previous_seasons: Vec<Season>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Settings {
    pub name: String,
    pub size: u32,
    pub schedule_settings: ScheduleSettings,
    pub scoring_settings: ScoringSettings,
    pub draft_settings: DraftSettings,
    pub trade_settings: TradeSettings,
    pub acquisition_settings: AcquisitionSettings,
    pub roster_settings: RosterSettings,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScheduleSettings {
    pub matchup_period_count: u32,
    pub matchup_periods: BTreeMap<MatchupPeriod, Vec<ScoringPeriod>>,
    pub playoff_team_count: u32,
    #[serde(default)]
    pub playoff_matchup_period_length: u32,
    pub playoff_seeding_rule: String,
    #[serde(default)]
    pub divisions: Vec<Division>,
}
#[derive(Deserialize)]
pub(super) struct Division {
    #[serde(default)]
    pub id: u32,
    pub name: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScoringSettings {
    pub matchup_tie_rule: String,
    pub playoff_matchup_tie_rule: String,
    pub scoring_type: Option<String>,
    pub scoring_enhancement_type: Option<String>,
    #[serde(default)]
    pub scoring_items: Vec<ScoringItem>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScoringItem {
    pub stat_id: StatId,
    #[serde(default)]
    pub points: f64,
    #[serde(default)]
    pub points_overrides: BTreeMap<SlotId, f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DraftSettings {
    pub keeper_count: u32,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TradeSettings {
    pub veto_votes_required: u32,
    pub deadline_date: Option<i64>,
    pub revision_hours: Option<f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AcquisitionSettings {
    pub is_using_acquisition_budget: bool,
    #[serde(default)]
    pub acquisition_budget: f64,
    pub acquisition_limit: Option<f64>,
    pub matchup_acquisition_limit: Option<f64>,
    pub matchup_limit_per_scoring_period: Option<bool>,
    #[serde(default)]
    pub minimum_bid: f64,
    #[serde(default)]
    pub waiver_process_days: Vec<String>,
    pub waiver_process_hour: Option<u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RosterSettings {
    #[serde(default)]
    pub lineup_slot_counts: BTreeMap<SlotId, u32>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Team {
    pub id: TeamId,
    pub abbrev: String,
    pub name: Option<String>,
    pub location: Option<String>,
    pub nickname: Option<String>,
    pub division_id: u32,
    pub record: Record,
    pub playoff_seed: u32,
    pub rank_final: Option<u32>,
    pub rank_calculated_final: Option<u32>,
    #[serde(default)]
    pub waiver_rank: u32,
    #[serde(default)]
    pub draft_day_projected_rank: u32,
    pub logo: Option<String>,
    #[serde(default)]
    pub owners: Vec<String>,
    #[serde(default)]
    pub roster: Roster,
    #[serde(default)]
    pub transaction_counter: Transactions,
    #[serde(default)]
    pub current_simulation_results: Simulation,
    #[serde(default)]
    pub values_by_stat: BTreeMap<StatId, f64>,
}
#[derive(Deserialize)]
pub(super) struct Record {
    pub overall: OverallRecord,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct OverallRecord {
    pub wins: u32,
    pub losses: u32,
    pub ties: u32,
    pub points_for: f64,
    pub points_against: f64,
    pub streak_length: u32,
    pub streak_type: String,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct Transactions {
    #[serde(default)]
    pub acquisitions: u32,
    #[serde(default)]
    pub acquisition_budget_spent: f64,
    #[serde(default)]
    pub drops: u32,
    #[serde(default)]
    pub trades: u32,
    #[serde(default, rename = "moveToIR")]
    pub move_to_ir: u32,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct Simulation {
    #[serde(default)]
    pub playoff_pct: f64,
}
#[derive(Deserialize, Default)]
pub(super) struct Roster {
    #[serde(default)]
    pub entries: Vec<RosterEntry>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct RosterEntry {
    pub player_id: Option<PlayerId>,
    pub lineup_slot_id: Option<SlotId>,
    pub acquisition_type: Option<String>,
    pub injury_status: Option<String>,
    pub player_pool_entry: PoolEntry,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PoolEntry {
    pub id: Option<PlayerId>,
    pub on_team_id: Option<TeamId>,
    pub player: Player,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Player {
    pub id: PlayerId,
    pub full_name: String,
    pub eligible_slots: Vec<SlotId>,
    pub pro_team_id: ProTeamId,
    pub positional_ranking: Option<u32>,
    pub jersey: Option<Value>,
    pub injury_status: Option<String>,
    #[serde(default)]
    pub injured: bool,
    #[serde(default)]
    pub ownership: Ownership,
    #[serde(default)]
    pub stats: Vec<PlayerStat>,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct Ownership {
    pub percent_owned: Option<f64>,
    pub percent_started: Option<f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PlayerStat {
    pub season_id: Option<Season>,
    pub stat_split_type_id: Option<u32>,
    pub scoring_period_id: ScoringPeriod,
    pub stat_source_id: Option<u32>,
    #[serde(default)]
    pub applied_total: f64,
    #[serde(default)]
    pub applied_average: f64,
    #[serde(default)]
    pub stats: BTreeMap<StatId, f64>,
    #[serde(default)]
    pub applied_stats: BTreeMap<StatId, f64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Matchup {
    pub id: Option<u64>,
    pub matchup_period_id: MatchupPeriod,
    pub home: Option<MatchupSide>,
    pub away: Option<MatchupSide>,
    pub winner: String,
    pub playoff_tier_type: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct MatchupSide {
    pub team_id: TeamId,
    pub total_points: Option<f64>,
}
#[derive(Deserialize)]
pub(super) struct Scoreboard {
    pub schedule: Vec<Value>,
}
/// Decode only the discriminator before parsing a selected scoreboard entry.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScoreboardPeriod {
    pub matchup_period_id: MatchupPeriod,
}
/// The Python scoreboard ignores winner entirely; expose it only when supplied.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScoreboardMatchup {
    pub id: Option<u64>,
    pub matchup_period_id: MatchupPeriod,
    pub home: Option<ScoreboardSide>,
    pub away: Option<ScoreboardSide>,
    pub winner: Option<String>,
    pub playoff_tier_type: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ScoreboardSide {
    pub team_id: TeamId,
    pub total_points: f64,
}

#[derive(Deserialize)]
pub(super) struct WeeklySchedule {
    pub schedule: Vec<WeeklyMatchup>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WeeklyMatchup {
    pub id: Option<u64>,
    pub playoff_tier_type: Option<String>,
    pub home: Option<Value>,
    pub away: Option<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WeeklySide {
    pub team_id: TeamId,
    pub roster_for_current_scoring_period: WeeklyRoster,
}
#[derive(Deserialize)]
pub(super) struct WeeklyRoster {
    pub entries: Vec<Value>,
}
#[derive(Deserialize)]
pub(super) struct ProSchedule {
    pub settings: ProSettings,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProSettings {
    pub pro_teams: Vec<ProTeamSchedule>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProTeamSchedule {
    pub id: ProTeamId,
    #[serde(default)]
    pub pro_games_by_scoring_period: BTreeMap<ScoringPeriod, Vec<Value>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ProGame {
    pub home_pro_team_id: ProTeamId,
    pub away_pro_team_id: ProTeamId,
    pub date: i64,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct PositionalRatings {
    #[serde(default)]
    pub position_against_opponent: PositionAgainstOpponent,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct PositionAgainstOpponent {
    #[serde(default)]
    pub positional_ratings: BTreeMap<u32, PositionRatings>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct PositionRatings {
    pub ratings_by_opponent: BTreeMap<ProTeamId, OpponentRating>,
}
#[derive(Deserialize)]
pub(super) struct OpponentRating {
    pub rank: u32,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WeeklyPlayerMetadata {
    pub default_position_id: Option<u32>,
    #[serde(default)]
    pub stats: Vec<WeeklyTeamEvidence>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct WeeklyTeamEvidence {
    pub scoring_period_id: Option<ScoringPeriod>,
    pub stat_source_id: Option<u32>,
    pub pro_team_id: Option<ProTeamId>,
}
