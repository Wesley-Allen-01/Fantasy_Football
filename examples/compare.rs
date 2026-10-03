//! Compare one team's weekly lineup with available players; no recommendations.
use espn_fantasy_football::{
    BoxPlayer, Client, Credentials, FreeAgentOptions, LeagueId, ScoringPeriod, Season, SlotId,
    TeamId,
    football::{pro_team_abbreviation, slot_label},
};
use std::{env, error::Error};

fn slot_name(slot: SlotId) -> String {
    slot_label(slot)
        .map(str::to_owned)
        .unwrap_or_else(|| slot.to_string())
}

fn print_players(players: &[BoxPlayer], week: ScoringPeriod) {
    println!(
        "{:<24} {:<7} {:<24} {:<5} {:<7} {:<14} {:>7} {:>8} {:>10}",
        "Player", "Pos", "Eligible", "NFL", "Opp", "Injury", "Owned%", "Actual", "Projected"
    );
    for player in players {
        let stats = player.player.stats.get(&week);
        let actual = stats
            .and_then(|stats| stats.actual.as_ref())
            .map(|stats| format!("{:.2}", stats.points))
            .unwrap_or_else(|| "—".into());
        let projected = stats
            .and_then(|stats| stats.projected.as_ref())
            .map(|stats| format!("{:.2}", stats.points))
            .unwrap_or_else(|| "—".into());
        let opponent = if player.on_bye_week {
            "BYE".into()
        } else {
            player
                .pro_opponent
                .map(|id| {
                    pro_team_abbreviation(id)
                        .map(str::to_owned)
                        .unwrap_or_else(|| id.to_string())
                })
                .unwrap_or_else(|| "—".into())
        };
        let owned = if player.player.percent_owned < 0.0 {
            "—".into()
        } else {
            format!("{:.2}", player.player.percent_owned)
        };
        println!(
            "{:<24} {:<7} {:<24} {:<5} {:<7} {:<14} {:>7} {:>8} {:>10}",
            player.player.name,
            player
                .player
                .position
                .map(slot_name)
                .unwrap_or_else(|| "—".into()),
            player
                .player
                .eligible_slots
                .iter()
                .copied()
                .map(slot_name)
                .collect::<Vec<_>>()
                .join(","),
            pro_team_abbreviation(player.pro_team)
                .map(str::to_owned)
                .unwrap_or_else(|| player.pro_team.to_string()),
            opponent,
            player.player.injury_status.as_deref().unwrap_or("—"),
            owned,
            actual,
            projected
        );
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().collect();
    if !(4..=8).contains(&args.len()) {
        return Err("usage: cargo run --example compare -- LEAGUE_ID SEASON TEAM_ID [WEEK|current] [SLOT|ALL] [LIMIT] [OFFSET]".into());
    }
    let league_id = LeagueId(args[1].parse()?);
    let season = Season(args[2].parse()?);
    let team_id = TeamId(args[3].parse()?);
    let week = args
        .get(4)
        .filter(|value| value.as_str() != "current")
        .map(|value| value.parse::<u32>().map(ScoringPeriod))
        .transpose()?;
    if week == Some(ScoringPeriod(0)) {
        return Err("an explicit week must be greater than zero".into());
    }
    let slots = match args.get(5).map(String::as_str) {
        None | Some("ALL") => Vec::new(),
        Some(value) => {
            let slot = value
                .parse::<u32>()
                .map(SlotId)
                .ok()
                .or_else(|| {
                    (0..=25).map(SlotId).find(|slot| {
                        slot_label(*slot).is_some_and(|label| {
                            !label.is_empty() && label.eq_ignore_ascii_case(value)
                        })
                    })
                })
                .ok_or_else(|| {
                    format!("unknown slot {value}; use a known label or numeric slot ID")
                })?;
            vec![slot]
        }
    };
    let limit = args
        .get(6)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(50_u32);
    let offset = args
        .get(7)
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(0_u32);
    if limit == 0 || offset.checked_add(limit).is_none() {
        return Err("page limit must be positive and offset plus limit must fit in u32".into());
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
        .ok_or_else(|| format!("team {team_id} was not found"))?;
    let weekly = league.box_scores(week).await?;
    let matchup = weekly
        .for_team(team_id)
        .ok_or_else(|| format!("no matchup found for team {team_id}"))?;
    let own = matchup
        .home
        .as_ref()
        .filter(|side| side.team_id == team_id)
        .or_else(|| matchup.away.as_ref().filter(|side| side.team_id == team_id))
        .ok_or("selected team has no lineup side")?;
    let available = league
        .free_agents(FreeAgentOptions {
            week: if weekly.scoring_period.0 == 0 {
                None
            } else {
                Some(weekly.scoring_period)
            },
            limit,
            offset,
            slots,
        })
        .await?;
    println!(
        "{} — {} — season {}, week {}",
        snapshot.settings.name, team.name, weekly.season, weekly.scoring_period
    );
    if week.is_some_and(|requested| requested != weekly.scoring_period) {
        println!("Requested future week; comparison uses the loaded current week for both groups.");
    }
    println!("\nYour weekly lineup (including bench and IR):");
    print_players(&own.lineup, weekly.scoring_period);
    println!(
        "\nAvailable players (free agents and waivers), offset {}:",
        available.offset
    );
    print_players(&available.players, available.scoring_period);
    println!("\n— means unavailable. Available players retain ESPN's ownership ordering.");
    if let Some(offset) = available.next_offset {
        println!("Next page offset: {offset} (advisory; the next page may be empty or repeat).");
    }
    Ok(())
}
