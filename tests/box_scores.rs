use espn_fantasy_football::football::{BoxScoreContext, PlayerTeamHistory, WeeklyBoxScores};
use espn_fantasy_football::{
    MatchupPeriod, PlayerId, ProTeamId, ScoringPeriod, Season, SlotId, StatId, TeamId,
};
use serde_json::{Value, json};

fn entry(id: i64, slot: Option<u32>, actual_team: Option<u32>, projected: f64) -> Value {
    let mut value = json!({
        "playerId":id,"acquisitionType":"DRAFT","injuryStatus":"NORMAL",
        "playerPoolEntry":{"id":id,"onTeamId":1,"player":{
            "id":id,"fullName":format!("Player {id}"),"eligibleSlots":[4,20,21],
            "proTeamId":16,"defaultPositionId":4,"injuryStatus":"ACTIVE",
            "stats":[{"seasonId":2024,"scoringPeriodId":7,"statSourceId":1,
                      "appliedTotal":projected,"stats":{"42":120},"appliedStats":{"42":projected}}]
        }}
    });
    if let Some(slot) = slot {
        value["lineupSlotId"] = json!(slot);
    }
    if let Some(team) = actual_team {
        value["playerPoolEntry"]["player"]["stats"]
            .as_array_mut()
            .unwrap()
            .insert(
                0,
                json!({
                    "seasonId":2024,"scoringPeriodId":7,"statSourceId":0,"proTeamId":team,
                    "appliedTotal":18.567,"stats":{"42":100},"appliedStats":{"42":18.567}
                }),
            );
    }
    value
}
fn boxes(entries: Vec<Value>) -> Value {
    json!({"schedule":[{"id":99,"home":{"teamId":1,"totalPoints":100.123,
        "rosterForCurrentScoringPeriod":{"entries":entries}},"playoffTierType":"WINNERS_BRACKET"}]})
}
fn schedule() -> Value {
    json!({"settings":{"proTeams":[
        {"id":11,"proGamesByScoringPeriod":{"7":[{"awayProTeamId":11,"homeProTeamId":12,"date":1000}, {"awayProTeamId":11,"homeProTeamId":20,"date":9999}]}},
        {"id":16,"proGamesByScoringPeriod":{"7":[{"awayProTeamId":16,"homeProTeamId":11,"date":2000}]}},
        {"id":0,"proGamesByScoringPeriod":{"7":[{}]}}
    ]}})
}
fn ratings() -> Value {
    json!({"positionAgainstOpponent":{"positionalRatings":{"4":{"ratingsByOpponent":{"12":{"rank":3}}}}}})
}
fn convert(
    boxes: &Value,
    pro: &Value,
    ranks: &Value,
    history: &mut PlayerTeamHistory,
    now: i64,
) -> espn_fantasy_football::Result<WeeklyBoxScores> {
    WeeklyBoxScores::from_values(
        boxes,
        pro,
        ranks,
        BoxScoreContext {
            season: Season(2024),
            scoring_period: ScoringPeriod(7),
            matchup_period: MatchupPeriod(4),
            now_unix_ms: now,
            history,
        },
    )
}

#[test]
fn weekly_stats_trade_correction_and_first_nfl_game_are_explicit() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let result = convert(
        &boxes(vec![entry(1, Some(4), Some(11), 20.25)]),
        &schedule(),
        &ratings(),
        &mut history,
        20_000_000,
    )
    .unwrap();
    assert_eq!(result.scoring_period, ScoringPeriod(7));
    assert_eq!(result.matchup_period, MatchupPeriod(4));
    let matchup = result.for_team(TeamId(1)).unwrap();
    assert!(matchup.is_playoff);
    assert!(matchup.away.is_none());
    assert!(result.for_team(TeamId(9)).is_none());
    let team = matchup.home.as_ref().unwrap();
    assert_eq!(team.score, 100.12);
    let player = &team.lineup[0];
    assert_eq!(player.points, 18.57);
    assert_eq!(player.projected_points, 20.25);
    assert_eq!(player.breakdown[&StatId(42)], 100.0);
    assert_eq!(player.points_breakdown[&StatId(42)], 18.567);
    assert_eq!(player.projected_breakdown[&StatId(42)], 120.0);
    assert_eq!(player.pro_team, ProTeamId(11));
    assert_eq!(player.player.pro_team, ProTeamId(11));
    assert_eq!(player.pro_opponent, Some(ProTeamId(12)));
    assert_eq!(player.pro_pos_rank, Some(3));
    assert_eq!(player.game_date_unix_ms, Some(1000));
    assert_eq!(player.game_played, 100);
    assert!(!player.on_bye_week);
    assert_eq!(history.get(PlayerId(1)), Some(ProTeamId(11)));
}

#[test]
fn live_score_and_projection_precedence_preserve_rounding_and_sentinel_behavior() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let mut data = boxes(vec![entry(1, Some(4), None, 20.25)]);
    data["schedule"][0]["home"]["totalPointsLive"] = json!(2.675);
    data["schedule"][0]["home"]["totalPoints"] = json!("ignored malformed non-live score");
    data["schedule"][0]["home"]["totalProjectedPointsLive"] = json!(-2.675);
    let result = convert(&data, &schedule(), &ratings(), &mut history, 0).unwrap();
    let home = result.matchups[0].home.as_ref().unwrap();
    assert_eq!(home.score, 2.67);
    assert_eq!(home.projected, -2.67);
    data["schedule"][0]["home"]["totalProjectedPointsLive"] = json!(-1.004);
    assert_eq!(
        convert(&data, &schedule(), &ratings(), &mut history, 0)
            .unwrap()
            .matchups[0]
            .home
            .as_ref()
            .unwrap()
            .projected,
        20.25
    );
    data["schedule"][0]["home"]
        .as_object_mut()
        .unwrap()
        .remove("totalProjectedPointsLive");
    assert_eq!(
        convert(&data, &schedule(), &ratings(), &mut history, 0)
            .unwrap()
            .matchups[0]
            .home
            .as_ref()
            .unwrap()
            .projected,
        20.25
    );
    data["schedule"][0]["home"]["totalProjectedPointsLive"] = Value::Null;
    assert!(convert(&data, &schedule(), &ratings(), &mut history, 0).is_err());
}

#[test]
fn fallback_projection_excludes_numeric_bench_and_ir_preserving_unknown_slots() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let mut data = boxes(vec![
        entry(1, Some(4), None, 10.125),
        entry(2, Some(20), None, 99.0),
        entry(3, Some(21), None, 88.0),
        entry(4, Some(777), None, 2.675),
        entry(5, None, None, 3.0),
    ]);
    data["schedule"][0]["home"]["totalProjectedPointsLive"] = json!(999); // ignored without live total
    let result = convert(&data, &schedule(), &ratings(), &mut history, 0).unwrap();
    let team = result.matchups[0].home.as_ref().unwrap();
    assert_eq!(team.projected, 10.12 + 2.67 + 3.0);
    assert_eq!(team.lineup[3].slot_position, Some(SlotId(777)));
    assert_eq!(team.lineup[3].slot_position_label(), None);
    assert_eq!(team.lineup[4].slot_position_label(), Some("FA"));
}

#[test]
fn game_progress_uses_strict_three_hour_boundary_and_missing_rank_semantics() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let data = boxes(vec![entry(1, Some(4), Some(11), 0.0)]);
    for (now, expected) in [(10_800_999, 0), (10_801_000, 0), (10_801_001, 100)] {
        let result = convert(&data, &schedule(), &ratings(), &mut history, now).unwrap();
        assert_eq!(
            result.matchups[0].home.as_ref().unwrap().lineup[0].game_played,
            expected
        );
    }
    let result = convert(&data, &schedule(), &json!({}), &mut history, 0).unwrap();
    let player = &result.matchups[0].home.as_ref().unwrap().lineup[0];
    assert_eq!(player.game_date_unix_ms, Some(1000));
    assert_eq!(player.pro_opponent, None);
    assert_eq!(player.pro_pos_rank, None);
    let ranks =
        json!({"positionAgainstOpponent":{"positionalRatings":{"4":{"ratingsByOpponent":{}}}}});
    let result = convert(&data, &schedule(), &ranks, &mut history, 0).unwrap();
    let player = &result.matchups[0].home.as_ref().unwrap().lineup[0];
    assert_eq!(player.pro_opponent, Some(ProTeamId(12)));
    assert_eq!(player.pro_pos_rank, None);
}

#[test]
fn cache_fallback_and_byes_do_not_create_new_evidence() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    history.insert(PlayerId(1), ProTeamId(11));
    let data = boxes(vec![
        entry(1, Some(4), None, 0.0),
        entry(2, Some(4), None, 0.0),
    ]);
    let before = history.clone();
    let result = convert(
        &data,
        &json!({"settings":{"proTeams":[]}}),
        &ratings(),
        &mut history,
        0,
    )
    .unwrap();
    let lineup = &result.matchups[0].home.as_ref().unwrap().lineup;
    assert_eq!(lineup[0].pro_team, ProTeamId(11));
    assert_eq!(lineup[1].pro_team, ProTeamId(16));
    assert!(lineup[0].on_bye_week);
    assert_eq!(lineup[0].game_played, 100);
    assert_eq!(lineup[0].game_date_unix_ms, None);
    assert_eq!(lineup[0].points, 0.0);
    assert_eq!(history, before);
}

#[test]
fn history_commits_only_after_complete_conversion_and_rejects_other_seasons() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    history.insert(PlayerId(9), ProTeamId(3));
    let before = history.clone();
    let mut data = boxes(vec![
        entry(1, Some(4), Some(11), 0.0),
        entry(2, Some(4), Some(12), 0.0),
    ]);
    data["schedule"][0]["home"]["rosterForCurrentScoringPeriod"]["entries"][1]["playerPoolEntry"]
        ["player"]["fullName"] = Value::Null;
    assert!(convert(&data, &schedule(), &ratings(), &mut history, 0).is_err());
    assert_eq!(history, before);
    let mut other = PlayerTeamHistory::new(Season(2023));
    assert!(convert(&boxes(vec![]), &schedule(), &ratings(), &mut other, 0).is_err());
    assert_eq!(other.season(), Season(2023));
    assert!(other.teams().is_empty());
}

#[test]
fn direct_card_wrapper_uses_explicit_top_level_metadata_fallback() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let original = entry(1, Some(4), Some(11), 20.0);
    let mut card = json!({"id":1,"fullName":"fallback name","eligibleSlots":[4,23],"jersey":"18","onTeamId":1,"lineupSlotId":4,"player":original["playerPoolEntry"]["player"]});
    card["player"]
        .as_object_mut()
        .unwrap()
        .remove("eligibleSlots");
    card["player"].as_object_mut().unwrap().remove("jersey");
    let result = convert(&boxes(vec![card]), &schedule(), &ratings(), &mut history, 0).unwrap();
    let player = &result.matchups[0].home.as_ref().unwrap().lineup[0].player;
    assert_eq!(player.name, "Player 1"); // nested identity wins
    assert_eq!(player.eligible_slots, vec![SlotId(4), SlotId(23)]);
    assert_eq!(player.jersey.as_deref(), Some("18"));
    assert_eq!(player.on_team_id, Some(TeamId(1)));
}

#[test]
fn first_matching_actual_nonzero_team_is_evidence_even_if_stats_are_filtered() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let mut item = entry(1, Some(4), Some(11), 20.0);
    let stats = item["playerPoolEntry"]["player"]["stats"]
        .as_array_mut()
        .unwrap();
    stats.insert(0,json!({"seasonId":2023,"scoringPeriodId":7,"statSourceId":0,"proTeamId":12,"appliedTotal":99}));
    let result = convert(&boxes(vec![item]), &schedule(), &ratings(), &mut history, 0).unwrap();
    let player = &result.matchups[0].home.as_ref().unwrap().lineup[0];
    assert_eq!(player.pro_team, ProTeamId(12));
    assert_eq!(player.points, 18.57);
    assert_eq!(history.get(PlayerId(1)), Some(ProTeamId(12)));
}

#[test]
fn preseason_default_week_zero_is_supported_without_rejecting_or_inferring_points() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let result = WeeklyBoxScores::from_values(
        &boxes(vec![entry(1, Some(4), None, 20.0)]),
        &schedule(),
        &ratings(),
        BoxScoreContext {
            season: Season(2024),
            scoring_period: ScoringPeriod(0),
            matchup_period: MatchupPeriod(0),
            now_unix_ms: 0,
            history: &mut history,
        },
    )
    .unwrap();
    assert_eq!(result.scoring_period, ScoringPeriod(0));
    let player = &result.matchups[0].home.as_ref().unwrap().lineup[0];
    assert_eq!(player.points, 0.0);
    assert_eq!(player.projected_points, 0.0);
    assert!(player.on_bye_week);
    assert!(history.teams().is_empty());
}

#[test]
fn extreme_timestamps_do_not_overflow_the_three_hour_boundary() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    let mut pro = schedule();
    pro["settings"]["proTeams"][0]["proGamesByScoringPeriod"]["7"][0]["date"] = json!(i64::MAX);
    let result = convert(
        &boxes(vec![entry(1, Some(4), Some(11), 0.0)]),
        &pro,
        &ratings(),
        &mut history,
        i64::MAX,
    )
    .unwrap();
    assert_eq!(
        result.matchups[0].home.as_ref().unwrap().lineup[0].game_played,
        0
    );
    pro["settings"]["proTeams"][0]["proGamesByScoringPeriod"]["7"][0]["date"] = json!(i64::MIN);
    let result = convert(
        &boxes(vec![entry(1, Some(4), Some(11), 0.0)]),
        &pro,
        &ratings(),
        &mut history,
        i64::MAX,
    )
    .unwrap();
    assert_eq!(
        result.matchups[0].home.as_ref().unwrap().lineup[0].game_played,
        100
    );
}

#[test]
fn direct_card_identity_mismatch_rejects_and_preserves_history_atomically() {
    let mut history = PlayerTeamHistory::new(Season(2024));
    history.insert(PlayerId(9), ProTeamId(3));
    let before = history.clone();
    let original = entry(2, Some(4), Some(12), 20.0);
    let card = json!({"id":1,"fullName":"outer name","lineupSlotId":4,"player":original["playerPoolEntry"]["player"]});
    let data = boxes(vec![entry(3, Some(4), Some(11), 10.0), card.clone()]);
    let error = convert(&data, &schedule(), &ratings(), &mut history, 0).unwrap_err();
    assert!(error.to_string().contains("roster player ID differs"));
    assert_eq!(history, before);
    let mut valid = card;
    valid["id"] = json!(2);
    let result = convert(
        &boxes(vec![valid]),
        &schedule(),
        &ratings(),
        &mut history,
        0,
    )
    .unwrap();
    assert_eq!(
        result.matchups[0].home.as_ref().unwrap().lineup[0]
            .player
            .name,
        "Player 2"
    );
    assert_eq!(history.get(PlayerId(2)), Some(ProTeamId(12)));
}
