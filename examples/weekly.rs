//! Read-only inspection of one team's weekly matchup and both lineups.
use espn_fantasy_football::{
    BoxTeam, Client, Credentials, LeagueId, LeagueSnapshot, ScoringPeriod, Season, TeamId,
    football::{pro_team_abbreviation, slot_label},
};
use std::{env, error::Error};

fn print_lineup(side: &BoxTeam, snapshot: &LeagueSnapshot) {
    let name = snapshot
        .team(side.team_id)
        .map(|team| team.name.as_str())
        .unwrap_or("Unknown team");
    println!(
        "\n{} (team {}): {:.2} points, {:.2} projected",
        name, side.team_id, side.score, side.projected
    );
    println!(
        "{:<8} {:<26} {:<6} {:<8} {:<14} {:>8} {:>10}",
        "Slot", "Player", "NFL", "Opponent", "Injury", "Points", "Projected"
    );
    for entry in &side.lineup {
        let slot = entry
            .slot_position
            .map(|slot| {
                slot_label(slot)
                    .map(str::to_owned)
                    .unwrap_or_else(|| slot.to_string())
            })
            .unwrap_or_else(|| "FA".into());
        let pro_team = pro_team_abbreviation(entry.pro_team)
            .map(str::to_owned)
            .unwrap_or_else(|| entry.pro_team.to_string());
        let opponent = if entry.on_bye_week {
            "BYE".into()
        } else {
            entry
                .pro_opponent
                .map(|id| {
                    pro_team_abbreviation(id)
                        .map(str::to_owned)
                        .unwrap_or_else(|| id.to_string())
                })
                .unwrap_or_else(|| "—".into())
        };
        println!(
            "{:<8} {:<26} {:<6} {:<8} {:<14} {:>8.2} {:>10.2}",
            slot,
            entry.player.name,
            pro_team,
            opponent,
            entry.player.injury_status.as_deref().unwrap_or("—"),
            entry.points,
            entry.projected_points
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if !(4..=5).contains(&args.len()) {
        return Err("usage: cargo run --example weekly -- LEAGUE_ID SEASON TEAM_ID [WEEK]".into());
    }
    let league_id = LeagueId(args[1].parse()?);
    let season = Season(args[2].parse()?);
    let team_id = TeamId(args[3].parse()?);
    let week = args
        .get(4)
        .map(|value| value.parse::<u32>().map(ScoringPeriod))
        .transpose()?;
    if week.is_some_and(|week| week.0 == 0) {
        return Err("an explicit week must be greater than zero".into());
    }
    let mut builder = Client::builder();
    match (env::var("ESPN_S2").ok(), env::var("ESPN_SWID").ok()) {
        (Some(s2), Some(swid)) => builder = builder.credentials(Credentials::new(s2, swid)?),
        (None, None) => {}
        _ => return Err("private league access requires both ESPN_S2 and ESPN_SWID".into()),
    }
    let mut league = builder.build()?.league(league_id, season)?;
    let snapshot = league.fetch().await?;
    let team = snapshot
        .team(team_id)
        .ok_or_else(|| format!("team {team_id} was not found in this league"))?;
    let weekly = league.box_scores(week).await?;
    println!(
        "{} — {} — season {}, week {}, matchup period {}",
        snapshot.settings.name,
        team.name,
        weekly.season,
        weekly.scoring_period,
        weekly.matchup_period
    );
    if week.is_some_and(|week| week != weekly.scoring_period) {
        println!("Requested future week; ESPN compatibility uses the loaded current week.");
    }
    let matchup = weekly.for_team(team_id).ok_or_else(|| {
        format!(
            "no matchup found for team {team_id} in week {}",
            weekly.scoring_period
        )
    })?;
    if matchup.is_playoff {
        println!("Playoff tier: {}", matchup.matchup_type);
    }
    let (own, opponent) = match matchup.home.as_ref().filter(|side| side.team_id == team_id) {
        Some(home) => (home, matchup.away.as_ref()),
        None => (
            matchup
                .away
                .as_ref()
                .ok_or("selected team has no lineup side")?,
            matchup.home.as_ref(),
        ),
    };
    print_lineup(own, &snapshot);
    if let Some(opponent) = opponent {
        print_lineup(opponent, &snapshot);
    } else {
        println!("\nFantasy matchup: BYE (no opposing team).");
    }
    Ok(())
}
