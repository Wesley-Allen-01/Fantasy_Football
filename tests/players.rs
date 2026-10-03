use espn_fantasy_football::football::{
    FreeAgentContext, FreeAgentOptions, FreeAgentPage, PlayerCard, PlayerDirectory,
};
use espn_fantasy_football::{PlayerId, ProTeamId, ScoringPeriod, Season, SlotId, StatId};
use serde_json::{Value, json};

fn wrapper(id: i64) -> Value {
    json!({"id":id,"onTeamId":0,"acquisitionType":"WAIVER","player":{
        "id":id,"fullName":format!("Player {id}"),"eligibleSlots":[0,20,777],"proTeamId":16,"defaultPositionId":1,
        "injuryStatus":"QUESTIONABLE","injured":true,"jersey":"18",
        "ownership":{"percentOwned":99.125,"percentStarted":2.675},
        "stats":[
            {"seasonId":2024,"scoringPeriodId":7,"statSourceId":0,"proTeamId":11,"appliedTotal":18.567,"stats":{"3":200,"22":20,"777":3},"appliedStats":{"4":18.567}},
            {"seasonId":2024,"scoringPeriodId":7,"statSourceId":1,"appliedTotal":20.125,"stats":{"3":250},"appliedStats":{"4":20.125}},
            {"seasonId":2023,"scoringPeriodId":7,"statSourceId":0,"appliedTotal":999},
            {"seasonId":2024,"scoringPeriodId":7,"statSourceId":0,"statSplitTypeId":2,"appliedTotal":999}
        ]
    }})
}
fn pro() -> Value {
    json!({"settings":{"proTeams":[
        {"id":16,"proGamesByScoringPeriod":{
            "7":[{"awayProTeamId":16,"homeProTeamId":23,"date":3000},{"awayProTeamId":16,"homeProTeamId":1,"date":6000}],
            "8":[{"awayProTeamId":12,"homeProTeamId":16,"date":9000}],"9":[]}},
        {"id":11,"proGamesByScoringPeriod":{"7":[{"awayProTeamId":11,"homeProTeamId":12,"date":1000}]}}
    ]}})
}
fn ranks() -> Value {
    json!({"positionAgainstOpponent":{"positionalRatings":{"1":{"ratingsByOpponent":{"12":{"rank":3}}}}}})
}
fn page(data: &Value, offset: u32, limit: u32) -> espn_fantasy_football::Result<FreeAgentPage> {
    FreeAgentPage::from_values(
        data,
        &pro(),
        &ranks(),
        FreeAgentContext {
            season: Season(2024),
            scoring_period: ScoringPeriod(7),
            now_unix_ms: 20_000_000,
            offset,
            limit,
        },
    )
}

#[test]
fn options_have_python_defaults_and_numeric_qb_slot_is_representable() {
    let mut options = FreeAgentOptions::default();
    assert_eq!(options.week, None);
    assert_eq!(options.limit, 50);
    assert_eq!(options.offset, 0);
    assert!(options.slots.is_empty());
    options.slots = vec![SlotId(0), SlotId(777)];
    assert_eq!(options.slots[0], SlotId(0));
}

#[test]
fn free_agents_preserve_order_weekly_fields_negative_ids_and_unknown_statistics() {
    let data = json!({"players":[wrapper(20),wrapper(-23),wrapper(5)]});
    let result = page(&data, 0, 3).unwrap();
    assert_eq!(
        result
            .players
            .iter()
            .map(|p| p.player.id)
            .collect::<Vec<_>>(),
        vec![PlayerId(20), PlayerId(-23), PlayerId(5)]
    );
    let player = &result.players[0];
    assert_eq!(player.points, 18.57);
    assert_eq!(player.projected_points, 20.12);
    assert_eq!(player.pro_team, ProTeamId(11));
    assert_eq!(player.pro_opponent, Some(ProTeamId(12)));
    assert_eq!(player.pro_pos_rank, Some(3));
    assert_eq!(player.game_date_unix_ms, Some(1000));
    assert_eq!(player.game_played, 100);
    assert_eq!(
        player.player.eligible_slots,
        vec![SlotId(0), SlotId(20), SlotId(777)]
    );
    assert_eq!(player.breakdown[&StatId(3)], 200.0);
    assert_eq!(player.breakdown[&StatId(22)], 20.0);
    assert_eq!(player.breakdown[&StatId(777)], 3.0);
    assert_eq!(player.player.percent_owned, 99.12);
    assert_eq!(player.player.percent_started, 2.67);
    assert_eq!(player.player.injury_status.as_deref(), Some("QUESTIONABLE"));
    assert_eq!(player.slot_position_label(), Some("FA"));
    assert_eq!(result.next_offset, Some(3));
}

#[test]
fn pagination_is_advisory_and_does_not_repeat_or_merge_pages() {
    let full = json!({"players":[wrapper(1),wrapper(2)]});
    let first = page(&full, 4, 2).unwrap();
    assert_eq!(first.next_offset, Some(6));
    let repeated = page(&full, 6, 2).unwrap();
    assert_eq!(repeated.next_offset, Some(8));
    assert_eq!(first.players[0].player.id, repeated.players[0].player.id);
    let short = page(&json!({"players":[wrapper(1)]}), 8, 2).unwrap();
    assert_eq!(short.next_offset, None);
    let empty = page(&json!({"players":[]}), 10, 2).unwrap();
    assert!(empty.players.is_empty());
    assert_eq!(empty.next_offset, None);
    assert!(page(&full, u32::MAX - 1, 2).is_err());
    assert!(page(&full, 0, 0).is_err());
    // Server may return more than the requested limit; check actual count too.
    assert!(page(&full, u32::MAX - 1, 1).is_err());
}

#[test]
fn malformed_and_duplicate_free_agent_players_are_rejected() {
    for data in [
        json!({}),
        json!({"players":null}),
        json!({"players":{}}),
        json!({"players":[wrapper(1),wrapper(1)]}),
        json!({"players":[wrapper(0)]}),
    ] {
        assert!(page(&data, 0, 50).is_err());
    }
    let mut invalid = wrapper(1);
    invalid["player"]["id"] = json!(2);
    assert!(page(&json!({"players":[invalid]}), 0, 50).is_err());
}

#[test]
fn free_agent_enrichment_has_no_history_and_missing_projection_stays_missing() {
    let mut current = wrapper(1);
    current["player"]["stats"] = json!([]);
    let result = page(&json!({"players":[current]}), 0, 50).unwrap();
    let player = &result.players[0];
    assert_eq!(player.pro_team, ProTeamId(16));
    assert_eq!(player.game_date_unix_ms, Some(3000));
    assert_eq!(player.points, 0.0);
    assert_eq!(player.projected_points, 0.0);
    assert!(player.player.stats.is_empty());
}

#[test]
fn cards_use_current_team_full_schedule_and_preserve_raw_untyped_history() {
    let mut raw = wrapper(1);
    raw["transactions"] = json!([{"type":"future_unknown_transaction","unimplemented":true}]);
    raw["customField"] = json!({"retained":"verbatim"});
    let cards =
        PlayerCard::from_values(&json!({"players":[raw.clone()]}), &pro(), Season(2024)).unwrap();
    let card = &cards[0];
    assert_eq!(card.player.pro_team, ProTeamId(16)); // weekly actual team11 does not replace it
    assert_eq!(card.schedule.len(), 2);
    assert_eq!(card.schedule[&ScoringPeriod(7)].opponent, ProTeamId(23));
    assert_eq!(card.schedule[&ScoringPeriod(7)].date_unix_ms, 3000);
    assert_eq!(card.schedule[&ScoringPeriod(8)].opponent, ProTeamId(12));
    assert!(!card.schedule.contains_key(&ScoringPeriod(9)));
    assert_eq!(card.raw, raw);
    assert_eq!(
        card.player.stats[&ScoringPeriod(7)]
            .actual
            .as_ref()
            .unwrap()
            .points,
        18.57
    );
    let absent = PlayerCard::from_values(
        &json!({"players":[wrapper(2)]}),
        &json!({"settings":{"proTeams":[]}}),
        Season(2024),
    )
    .unwrap();
    assert!(absent[0].schedule.is_empty());
}

#[test]
fn cards_ignore_unused_weekly_team_and_position_metadata() {
    let mut raw = wrapper(1);
    raw["player"]["stats"][0]["proTeamId"] = json!({"unused":"malformed weekly metadata"});
    raw["player"]["defaultPositionId"] = json!({"unused":"malformed position metadata"});
    let cards = PlayerCard::from_values(&json!({"players":[raw]}), &pro(), Season(2024)).unwrap();
    assert_eq!(cards[0].player.id, PlayerId(1));
    assert_eq!(cards[0].player.pro_team, ProTeamId(16));
}

#[test]
fn cards_reject_missing_duplicate_and_conflicting_ids_without_merging() {
    for data in [
        json!({}),
        json!({"players":null}),
        json!({"players":[wrapper(1),wrapper(1)]}),
        json!({"players":[wrapper(0)]}),
    ] {
        assert!(PlayerCard::from_values(&data, &pro(), Season(2024)).is_err());
    }
    let mut conflicting = wrapper(1);
    conflicting["player"]["id"] = json!(2);
    assert!(
        PlayerCard::from_values(&json!({"players":[conflicting]}), &pro(), Season(2024)).is_err()
    );
    assert!(
        PlayerCard::from_values(&json!({"players":[]}), &pro(), Season(2024))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn directory_preserves_duplicate_names_exact_matching_and_server_order() {
    let data = json!([
        {"id":3,"fullName":"Same Name"},
        {"id":1,"fullName":"Other"},
        {"id":-23,"fullName":"Same Name"},
        {"id":3,"fullName":"Same Name"}
    ]);
    let directory = PlayerDirectory::from_value(&data, Season(2024)).unwrap();
    assert_eq!(directory.players.len(), 3);
    assert_eq!(
        directory.ids_named("Same Name"),
        vec![PlayerId(3), PlayerId(-23)]
    );
    assert!(directory.ids_named("same name").is_empty());
    assert!(directory.ids_named("Unknown").is_empty());
    assert_eq!(directory.players[1].id, PlayerId(1));
}

#[test]
fn directory_requires_a_valid_unambiguous_identity() {
    for data in [
        json!({"players":[]}),
        json!(null),
        json!([{"fullName":"Missing ID"}]),
        json!([{"id":1}]),
        json!([{"id":0,"fullName":"Invalid"}]),
        json!([{"id":1,"fullName":" "}]),
        json!([{"id":1,"fullName":"First"},{"id":1,"fullName":"Different"}]),
    ] {
        assert!(PlayerDirectory::from_value(&data, Season(2024)).is_err());
    }
    assert!(
        PlayerDirectory::from_value(&json!([]), Season(2024))
            .unwrap()
            .players
            .is_empty()
    );
}

#[test]
fn card_positional_rank_uses_known_wrapper_path_with_explicit_precedence() {
    let mut raw = wrapper(1);
    raw["ratings"] = json!({"0":{"positionalRanking":35}});
    let parse = |raw: &Value| {
        PlayerCard::from_values(&json!({"players":[raw]}), &pro(), Season(2024))
            .unwrap()
            .remove(0)
    };
    assert_eq!(parse(&raw).player.positional_rank, Some(35));
    raw["positionalRanking"] = json!(42);
    assert_eq!(parse(&raw).player.positional_rank, Some(42));
    raw["player"]["positionalRanking"] = json!(7);
    assert_eq!(parse(&raw).player.positional_rank, Some(7));
    let mut pool = json!({"playerId":1,"playerPoolEntry":{"id":1,"player":raw["player"],"ratings":{"0":{"positionalRanking":99}}}});
    pool["playerPoolEntry"]["player"]
        .as_object_mut()
        .unwrap()
        .remove("positionalRanking");
    assert_eq!(parse(&pool).player.positional_rank, Some(99));
}

#[test]
fn cards_tolerate_missing_schedule_objects_without_weakening_weekly_reads() {
    let cards = json!({"players":[wrapper(1)]});
    for schedule in [
        json!({}),
        json!({"settings":{}}),
        json!({"settings":{"proTeams":[]}}),
        json!({"settings":{"proTeams":{}}}),
    ] {
        let result = PlayerCard::from_values(&cards, &schedule, Season(2024)).unwrap();
        assert!(result[0].schedule.is_empty());
    }
    for schedule in [json!({}), json!({"settings":{}})] {
        assert!(
            FreeAgentPage::from_values(
                &cards,
                &schedule,
                &ranks(),
                FreeAgentContext {
                    season: Season(2024),
                    scoring_period: ScoringPeriod(7),
                    now_unix_ms: 0,
                    offset: 0,
                    limit: 50
                }
            )
            .is_err()
        );
    }
}

#[test]
fn cards_reject_null_and_malformed_schedule_objects_instead_of_defaulting() {
    let cards = json!({"players":[wrapper(1)]});
    for schedule in [
        json!(null),
        json!([]),
        json!({"settings":null}),
        json!({"settings":[]}),
        json!({"settings":1}),
        json!({"settings":{"proTeams":null}}),
        json!({"settings":{"proTeams":1}}),
        json!({"settings":{"proTeams":{"16":{}}}}),
        json!({"settings":{"proTeams":[null]}}),
    ] {
        assert!(PlayerCard::from_values(&cards, &schedule, Season(2024)).is_err());
    }
}
