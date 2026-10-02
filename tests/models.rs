use espn_fantasy_football::football::{
    ActiveStatus, Outcome, pro_team_abbreviation, scoreboard_from_value, slot_label,
};
use espn_fantasy_football::{
    LeagueId, LeagueSnapshot, MatchupPeriod, PlayerId, ScoringPeriod, Season, SlotId, StatId,
    TeamId,
};
use serde_json::{Value, json};

fn fixture(year: u16) -> Value {
    let text = match year {
        2015 => include_str!("fixtures/football_2015_league.json"),
        _ => include_str!("fixtures/football_2018_league.json"),
    };
    let value: Value = serde_json::from_str(text).unwrap();
    if value.is_array() {
        value[0].clone()
    } else {
        value
    }
}
fn snapshot(value: &Value, year: u16) -> LeagueSnapshot {
    LeagueSnapshot::from_value(value, LeagueId(value["id"].as_u64().unwrap()), Season(year))
        .unwrap()
}

#[test]
fn historical_and_modern_payloads_decode_to_owned_id_linked_snapshots() {
    for year in [2015, 2018] {
        let data = fixture(year);
        let league = snapshot(&data, year);
        assert_eq!(league.teams.len(), data["teams"].as_array().unwrap().len());
        assert!(
            league
                .teams
                .windows(2)
                .all(|teams| teams[0].id < teams[1].id)
        );
        assert_eq!(
            league.teams[0].points_against,
            if year == 2018 { 1640.6 } else { 1298.0 }
        );
        for team in &league.teams {
            for entry in &team.schedule {
                if let Some(opponent) = entry.opponent {
                    assert!(league.team(opponent).is_some());
                }
            }
        }
        assert!(
            league
                .previous_seasons
                .iter()
                .all(|previous| previous.0 < year)
        );
        let encoded = serde_json::to_value(&league).unwrap();
        assert_eq!(encoded["season"], year);
    }
}

#[test]
fn modern_current_week_is_capped_but_historical_week_is_not() {
    for year in [2015, 2018] {
        let mut data = fixture(year);
        data["scoringPeriodId"] = json!(99);
        data["status"]["finalScoringPeriod"] = json!(16);
        let league = snapshot(&data, year);
        assert_eq!(
            league.current_week,
            ScoringPeriod(if year < 2018 { 99 } else { 16 })
        );
        assert_eq!(league.scoring_period, ScoringPeriod(99));
    }
}

#[test]
fn slot_counts_use_ids_and_scoring_rules_are_independent_between_snapshots() {
    let mut data = fixture(2018);
    data["settings"]["rosterSettings"]["lineupSlotCounts"] = json!({"23": 2, "0": 1, "777": 3});
    data["settings"]["scoringSettings"]["scoringItems"] = json!([
        {"statId": 4,"points":4,"pointsOverrides":{"16":0}},
        {"statId": 777,"points":-2},
        {"statId": 4,"points":6}
    ]);
    let league = snapshot(&data, 2018);
    assert_eq!(league.settings.position_slot_counts[&SlotId(23)], 2);
    assert_eq!(league.settings.position_slot_counts[&SlotId(777)], 3);
    assert_eq!(slot_label(SlotId(777)), None);
    assert_eq!(league.settings.scoring_format[0].points, 0.0);
    assert_eq!(
        league.settings.scoring_format[0].points_overrides[&SlotId(16)],
        0.0
    );
    assert_eq!(league.settings.scoring_format[1].label, "Unknown");
    assert_eq!(league.settings.scoring_format[2].points, 6.0);
    data["settings"]["scoringSettings"]["scoringItems"][0]["points"] = json!(9);
    let other = snapshot(&data, 2018);
    assert_eq!(other.settings.scoring_format[0].points, 0.0);
    assert_eq!(other.settings.scoring_format[0].base_points, 9.0);
    assert_eq!(league.settings.scoring_format[0].points, 0.0);
}

#[test]
fn player_stats_keep_actual_projection_seasons_and_split_types_distinct() {
    let mut data = fixture(2018);
    let player = &mut data["teams"][0]["roster"]["entries"][0]["playerPoolEntry"]["player"];
    let player_id = PlayerId(player["id"].as_i64().unwrap());
    player["stats"] = json!([
        {"seasonId":2017,"scoringPeriodId":0,"statSourceId":0,"appliedTotal":999},
        {"seasonId":2018,"scoringPeriodId":0,"statSourceId":0,"statSplitTypeId":2,"appliedTotal":999},
        {"seasonId":2018,"scoringPeriodId":0,"statSourceId":0,"appliedTotal":2.675,"appliedAverage":2.685,"stats":{"3":1,"22":2,"777":3},"appliedStats":{"4":8}},
        {"seasonId":2018,"scoringPeriodId":0,"statSourceId":1,"appliedTotal":-2.675,"appliedAverage":1.125},
        {"seasonId":2018,"scoringPeriodId":1,"statSourceId":0,"appliedTotal":1.005,"stats":{}}
    ]);
    player["injuryStatus"] = json!("QUESTIONABLE");
    player["ownership"] = json!({"percentOwned":2.675,"percentStarted":2.685});
    let league = snapshot(&data, 2018);
    let player = league.teams[0].player(player_id).unwrap();
    assert_eq!(player.total_points, 2.67);
    assert_eq!(player.avg_points, 2.69);
    assert_eq!(player.projected_total_points, -2.67);
    assert_eq!(player.projected_avg_points, 1.12);
    assert_eq!(player.percent_owned, 2.67);
    assert_eq!(player.percent_started, 2.69);
    assert_eq!(player.injury_status.as_deref(), Some("QUESTIONABLE"));
    assert_eq!(player.active_status, ActiveStatus::Inactive);
    let actual = player.stats[&ScoringPeriod(0)].actual.as_ref().unwrap();
    assert_eq!(actual.breakdown.len(), 3);
    assert_eq!(actual.breakdown[&StatId(3)], 1.0);
    assert_eq!(actual.breakdown[&StatId(22)], 2.0);
    assert_eq!(actual.points_breakdown[&StatId(4)], 8.0);
    assert_eq!(
        player.stats[&ScoringPeriod(1)]
            .actual
            .as_ref()
            .unwrap()
            .points,
        1.0
    );
}

#[test]
fn bye_and_missing_score_are_explicit_and_margins_use_the_matchup_sides() {
    let mut data = fixture(2018);
    let id = TeamId(data["teams"][0]["id"].as_u64().unwrap() as u32);
    let other = TeamId(data["teams"][1]["id"].as_u64().unwrap() as u32);
    data["schedule"] = json!([
        {"id":1,"matchupPeriodId":1,"home":{"teamId":id,"totalPoints":10},"winner":"UNDECIDED"},
        {"id":2,"matchupPeriodId":2,"away":{"teamId":id,"totalPoints":15},"home":{"teamId":other,"totalPoints":12},"winner":"AWAY","playoffTierType":"WINNERS_BRACKET"},
        {"id":3,"matchupPeriodId":3,"home":{"teamId":id},"away":{"teamId":other,"totalPoints":4},"winner":"TIE"}
    ]);
    let league = snapshot(&data, 2018);
    let team = league.team(id).unwrap();
    assert_eq!(team.schedule[0].opponent, None);
    assert_eq!(team.schedule[0].margin, None);
    assert_eq!(team.schedule[0].outcome, Outcome::Undecided);
    assert_eq!(team.schedule[1].margin, Some(3.0));
    assert_eq!(team.schedule[1].outcome, Outcome::Win);
    assert_eq!(team.schedule[2].score, None);
    assert_eq!(team.schedule[2].outcome, Outcome::Tie);
    let scoreboard = scoreboard_from_value(&data, MatchupPeriod(1)).unwrap();
    assert_eq!(scoreboard.len(), 1);
    assert_eq!(scoreboard[0].away_team, None);
    assert_eq!(scoreboard[0].away_score, None);
    assert!(scoreboard_from_value(&data, MatchupPeriod(2)).unwrap()[0].is_playoff);
}

#[test]
fn standings_use_final_rank_then_seed_and_owner_records_follow_members_order() {
    let mut data = fixture(2018);
    data["teams"] = json!([data["teams"][0], data["teams"][1]]);
    data["members"] = json!([{"id":"b","displayName":"B"},{"id":"a","displayName":"A"}]);
    data["teams"][0]["owners"] = json!(["a", "b", "missing"]);
    data["teams"][0]["rankFinal"] = json!(0);
    data["teams"][0]["rankCalculatedFinal"] = json!(0);
    data["teams"][0]["playoffSeed"] = json!(2);
    data["teams"][1]["rankFinal"] = json!(1);
    let league = snapshot(&data, 2018);
    assert_eq!(league.standings()[0].final_standing, Some(1));
    assert_eq!(league.teams[0].owners[0]["id"], "b");
    assert_eq!(league.teams[0].owners[1]["id"], "a");
    assert_eq!(league.teams[0].owner_ids.len(), 3);
}

#[test]
fn invalid_ids_and_historical_envelopes_return_errors() {
    let data = fixture(2018);
    assert!(LeagueSnapshot::from_value(&data, LeagueId(0), Season(2018)).is_err());
    assert!(
        LeagueSnapshot::from_value(&data, LeagueId(data["id"].as_u64().unwrap()), Season(2019))
            .is_err()
    );
    assert!(scoreboard_from_value(&json!([]), MatchupPeriod(1)).is_err());
    assert!(scoreboard_from_value(&json!([data.clone(), data.clone()]), MatchupPeriod(1)).is_err());
    let mut data = data;
    data["teams"][1]["id"] = data["teams"][0]["id"].clone();
    assert!(
        LeagueSnapshot::from_value(&data, LeagueId(data["id"].as_u64().unwrap()), Season(2018))
            .is_err()
    );
}

#[test]
fn defenses_preserve_negative_ids_and_position_labels() {
    let mut data = fixture(2018);
    let entry = &mut data["teams"][0]["roster"]["entries"][0];
    entry["playerId"] = json!(-23);
    entry["playerPoolEntry"]["id"] = json!(-23);
    let player = &mut entry["playerPoolEntry"]["player"];
    player["id"] = json!(-23);
    player["fullName"] = json!("Steelers D/ST");
    player["eligibleSlots"] = json!([16, 20, 21]);
    player["proTeamId"] = json!(23);
    let league = snapshot(&data, 2018);
    let defense = league.teams[0].player(PlayerId(-23)).unwrap();
    assert_eq!(defense.position, Some(SlotId(16)));
    assert_eq!(slot_label(defense.position.unwrap()), Some("D/ST"));
    assert_eq!(pro_team_abbreviation(defense.pro_team), Some("PIT"));
}

#[test]
fn nfl_week_keeps_latest_period_distinct_and_tolerates_missing_metadata() {
    for year in [2015, 2018] {
        let mut data = fixture(year);
        let league = snapshot(&data, year);
        assert_eq!(league.nfl_week, Some(ScoringPeriod(18)));
        assert_eq!(league.current_week, ScoringPeriod(16));
        data["status"]["latestScoringPeriod"] = json!(20);
        assert_eq!(snapshot(&data, year).nfl_week, Some(ScoringPeriod(20)));
        data["status"]
            .as_object_mut()
            .unwrap()
            .remove("latestScoringPeriod");
        assert_eq!(snapshot(&data, year).nfl_week, None);
    }
}

#[test]
fn scoreboard_filters_before_decoding_and_accepts_absent_winner_metadata() {
    let data = json!({"schedule":[
        {"matchupPeriodId":1,"home":{"teamId":1,"totalPoints":12.5}},
        {"matchupPeriodId":2,"id":"invalid unrelated id","home":{"teamId":"invalid unrelated team"},"winner":99},
        {"matchupPeriodId":2,"home":{"totalPoints":"invalid unrelated score"}},
        {"matchupPeriodId":1,"home":{"teamId":2,"totalPoints":5},"away":{"teamId":3,"totalPoints":4},"winner":null}
    ]});
    let matchups = scoreboard_from_value(&data, MatchupPeriod(1)).unwrap();
    assert_eq!(matchups.len(), 2);
    assert_eq!(matchups[0].winner, None);
    assert_eq!(matchups[0].home_score, Some(12.5));
    assert_eq!(matchups[0].away_team, None);
    assert_eq!(matchups[0].away_score, None);
    assert_eq!(matchups[1].winner, None);
    assert_eq!(matchups[1].away_team, Some(TeamId(3)));
    assert!(scoreboard_from_value(&data, MatchupPeriod(2)).is_err());
    let missing_score = json!({"schedule":[{"matchupPeriodId":1,"home":{"teamId":1}}]});
    assert!(scoreboard_from_value(&missing_score, MatchupPeriod(1)).is_err());
}

#[test]
fn absent_and_null_stat_source_update_active_status_but_store_projected_points() {
    for source in [None, Some(Value::Null)] {
        for (breakdown, status) in [
            (json!({}), ActiveStatus::Inactive),
            (json!({"3":10}), ActiveStatus::Active),
        ] {
            let mut data = fixture(2018);
            let mut line =
                json!({"seasonId":2018,"scoringPeriodId":0,"appliedTotal":12,"stats":breakdown});
            if let Some(source) = &source {
                line["statSourceId"] = source.clone();
            }
            data["teams"][0]["roster"]["entries"][0]["playerPoolEntry"]["player"]["stats"] = json!([
                line,
                {"seasonId":2018,"scoringPeriodId":1,"statSourceId":1,"stats":{},"appliedTotal":20}
            ]);
            let league = snapshot(&data, 2018);
            let player = &league.teams[0].roster[0];
            assert_eq!(player.active_status, status);
            assert_eq!(player.total_points, 0.0);
            assert_eq!(player.projected_total_points, 12.0);
            assert!(player.stats[&ScoringPeriod(0)].actual.is_none());
            assert_eq!(
                player.stats[&ScoringPeriod(0)]
                    .projected
                    .as_ref()
                    .unwrap()
                    .points,
                12.0
            );
        }
    }
}
