use espn_fantasy_football::{Client, Credentials, LeagueId, PlayerId, Season};
use std::{env, error::Error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 5 || !matches!(args[3].as_str(), "id" | "name") {
        return Err("usage: cargo run --example lookup -- LEAGUE_ID SEASON id PLAYER_ID | LEAGUE_ID SEASON name \"EXACT NAME\"".into());
    }
    let id = if args[3] == "id" {
        Some(PlayerId(args[4].parse()?))
    } else {
        None
    };
    if id == Some(PlayerId(0)) || (id.is_none() && args[4].trim().is_empty()) {
        return Err("player ID must be nonzero and name must not be blank".into());
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
    league.fetch().await?;
    let cards = if let Some(id) = id {
        league.player_by_id(id).await?.into_iter().collect()
    } else {
        league.players_named(&args[4]).await?
    };
    if cards.is_empty() {
        println!("No matching player found.");
    } else {
        println!("{}", serde_json::to_string_pretty(&cards)?);
    }
    Ok(())
}
