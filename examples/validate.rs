//! Driver for scripts/validate_live.py. No network in `project` mode.
use espn_fantasy_football::{
    BoxPlayer, BoxScoreContext, BoxTeam, Client, FreeAgentContext, FreeAgentOptions, FreeAgentPage,
    LeagueId, LeagueSnapshot, MatchupPeriod, Player, PlayerCard, PlayerId, PlayerTeamHistory,
    ScoringPeriod, Season, TeamId, WeeklyBoxScores,
    football::{pro_team_abbreviation, slot_label},
};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use std::{
    env,
    error::Error,
    io::{self, Read},
    time::Duration,
};

#[derive(Clone, Deserialize)]
struct Config {
    league_id: u64,
    season: u16,
    team_id: u32,
    week: Option<u32>,
    limit: u32,
    offset: u32,
    player_id: Option<i64>,
}

#[derive(Deserialize)]
struct Context {
    scoring_period: u32,
    matchup_period: u32,
    player_id: i64,
    now_unix_ms: i64,
}

#[derive(Deserialize)]
struct Replay {
    config: Config,
    context: Context,
    league: Value,
    weekly: Value,
    weekly_schedule: Value,
    weekly_ratings: Value,
    free_agents: Value,
    free_agents_schedule: Value,
    free_agents_ratings: Value,
    cards: Value,
    cards_schedule: Value,
}

fn player_projection(p: &Player) -> Value {
    let mut stats = Map::new();
    for (period, lines) in &p.stats {
        let mut fields = Map::new();
        for (name, line) in [("actual", &lines.actual), ("projected", &lines.projected)] {
            if let Some(line) = line {
                fields.insert(
                    name.into(),
                    json!({"points":line.points, "average_points":line.average_points}),
                );
            }
        }
        stats.insert(period.to_string(), Value::Object(fields));
    }
    json!({
        "id":p.id, "name":p.name, "position":p.position.and_then(slot_label),
        "eligible_slots":p.eligible_slots.iter().map(|id| slot_label(*id)).collect::<Vec<_>>(),
        "pro_team":pro_team_abbreviation(p.pro_team),
        "total_points":p.total_points, "projected_total_points":p.projected_total_points,
        "avg_points":p.avg_points, "projected_avg_points":p.projected_avg_points,
        "injury_status":p.injury_status, "injured":p.injured,
        "percent_owned":p.percent_owned, "percent_started":p.percent_started, "stats":stats,
    })
}

fn box_player_projection(p: &BoxPlayer) -> Value {
    json!({
        "player":player_projection(&p.player), "slot_position":p.slot_position_label(),
        "pro_team":pro_team_abbreviation(p.pro_team),
        "pro_opponent":p.pro_opponent.and_then(pro_team_abbreviation),
        "pro_pos_rank":p.pro_pos_rank, "game_date_unix_ms":p.game_date_unix_ms,
        "game_played":p.game_played, "on_bye_week":p.on_bye_week,
        "points":p.points, "projected_points":p.projected_points,
    })
}

fn side_projection(side: &BoxTeam) -> Value {
    json!({"team_id":side.team_id, "score":side.score, "projected":side.projected,
        "lineup":side.lineup.iter().map(box_player_projection).collect::<Vec<_>>()})
}

fn project(input: Replay) -> Result<Value, Box<dyn Error>> {
    let c = &input.config;
    let ctx = &input.context;
    let season = Season(c.season);
    let snapshot = LeagueSnapshot::from_value(&input.league, LeagueId(c.league_id), season)?;
    let team = snapshot
        .team(TeamId(c.team_id))
        .ok_or("selected team not found")?;
    let mut history = PlayerTeamHistory::new(season);
    let weekly = WeeklyBoxScores::from_values(
        &input.weekly,
        &input.weekly_schedule,
        &input.weekly_ratings,
        BoxScoreContext {
            season,
            scoring_period: ScoringPeriod(ctx.scoring_period),
            matchup_period: MatchupPeriod(ctx.matchup_period),
            now_unix_ms: ctx.now_unix_ms,
            history: &mut history,
        },
    )?;
    if weekly.for_team(TeamId(c.team_id)).is_none() {
        return Err("selected team has no weekly matchup".into());
    }
    let available = FreeAgentPage::from_values(
        &input.free_agents,
        &input.free_agents_schedule,
        &input.free_agents_ratings,
        FreeAgentContext {
            season,
            scoring_period: ScoringPeriod(ctx.scoring_period),
            now_unix_ms: ctx.now_unix_ms,
            offset: c.offset,
            limit: c.limit,
        },
    )?;
    let cards = PlayerCard::from_values(&input.cards, &input.cards_schedule, season)?;
    if cards
        .iter()
        .any(|card| card.player.id != PlayerId(ctx.player_id))
    {
        return Err("unexpected player card identity".into());
    }
    Ok(json!({
        "league": {"id":snapshot.league_id, "season":snapshot.season,
            "name":snapshot.settings.name, "current_week":snapshot.current_week,
            "current_matchup_period":snapshot.current_matchup_period},
        "team": {"id":team.id, "name":team.name,
            "roster":team.roster.iter().map(player_projection).collect::<Vec<_>>()},
        "weekly": {"season":weekly.season, "scoring_period":weekly.scoring_period,
            "matchup_period":weekly.matchup_period,
            "matchups":weekly.matchups.iter().map(|m| json!({"id":m.id,
                "home":m.home.as_ref().map(side_projection), "away":m.away.as_ref().map(side_projection),
                "matchup_type":m.matchup_type, "is_playoff":m.is_playoff})).collect::<Vec<_>>()},
        "free_agents":available.players.iter().map(box_player_projection).collect::<Vec<_>>(),
        "cards":cards.iter().map(|card|json!({"player":player_projection(&card.player),
            "schedule":card.schedule.iter().map(|(period,game)|(period.to_string(),json!({
                "opponent":pro_team_abbreviation(game.opponent), "date_unix_ms":game.date_unix_ms
            }))).collect::<Map<_,_>>()})).collect::<Vec<_>>()
    }))
}

async fn read(c: Config, base: &str) -> Result<Value, Box<dyn Error>> {
    // The Python recorder holds cookies and forwards only approved ESPN reads.
    // This driver talks only to its loopback server and never reads credentials.
    let url = reqwest::Url::parse(base)?;
    if url.host_str() != Some("127.0.0.1") || url.scheme() != "http" {
        return Err("validation read requires the loopback recorder".into());
    }
    if c.season < 2019
        || c.team_id == 0
        || c.limit == 0
        || c.offset.checked_add(c.limit).is_none()
        || c.week == Some(0)
        || c.player_id == Some(0)
    {
        return Err("invalid validation options".into());
    }
    let mut league = Client::builder()
        .base_url(base)
        .timeout(Duration::from_secs(40))
        .build()?
        .league(LeagueId(c.league_id), Season(c.season))?;
    let snapshot = league.fetch().await?;
    let team = snapshot
        .team(TeamId(c.team_id))
        .ok_or("selected team not found")?;
    let player_id = c
        .player_id
        .map(PlayerId)
        .or_else(|| team.roster.first().map(|p| p.id))
        .ok_or("empty roster; supply a player ID")?;
    let weekly = league.box_scores(c.week.map(ScoringPeriod)).await?;
    if weekly.for_team(TeamId(c.team_id)).is_none() {
        return Err("selected team has no weekly matchup".into());
    }
    let available = league
        .free_agents(FreeAgentOptions {
            week: (weekly.scoring_period.0 != 0).then_some(weekly.scoring_period),
            limit: c.limit,
            offset: c.offset,
            slots: vec![],
        })
        .await?;
    let card = league.player_by_id(player_id).await?;
    Ok(
        json!({"scoring_period":weekly.scoring_period, "matchup_period":weekly.matchup_period,
        "player_id":player_id, "counts":{"roster":team.roster.len(),
        "matchups":weekly.matchups.len(), "available":available.players.len(), "cards":usize::from(card.is_some())}}),
    )
}

async fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let output = match args.get(1).map(String::as_str) {
        Some("project") if args.len() == 2 => project(serde_json::from_str(&input)?)?,
        Some("read") if args.len() == 3 => read(serde_json::from_str(&input)?, &args[2]).await?,
        _ => {
            return Err(
                "use scripts/validate_live.py to capture or verify a response bundle".into(),
            );
        }
    };
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        // Payloads and exception text may contain private league information.
        // Detailed access categories are recorded by the Python recorder.
        eprintln!("validation driver failed; inspect the local validation report");
        std::process::exit(1);
    }
}
