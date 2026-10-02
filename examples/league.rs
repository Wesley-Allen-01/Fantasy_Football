use espn_fantasy_football::{Client, Credentials, LeagueId, Season};
use std::{env, error::Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        return Err("usage: cargo run --example league -- LEAGUE_ID SEASON".into());
    }
    let mut builder = Client::builder();
    match (env::var("ESPN_S2").ok(), env::var("ESPN_SWID").ok()) {
        (Some(s2), Some(swid)) => builder = builder.credentials(Credentials::new(s2, swid)?),
        (None, None) => {}
        _ => return Err("private league access requires both ESPN_S2 and ESPN_SWID".into()),
    }
    let mut league = builder
        .build()?
        .league(LeagueId(args[1].parse()?), Season(args[2].parse()?))?;
    let snapshot = league.fetch().await?;
    println!(
        "{} ({} teams, scoring period {})",
        snapshot.settings.name,
        snapshot.teams.len(),
        snapshot.current_week
    );
    println!(
        "Standings:\n{}",
        serde_json::to_string_pretty(&snapshot.standings())?
    );
    println!(
        "Scoreboard:\n{}",
        serde_json::to_string_pretty(&league.scoreboard(None).await?)?
    );
    Ok(())
}
