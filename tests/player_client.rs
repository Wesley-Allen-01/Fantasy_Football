use espn_fantasy_football::{
    Client, Error, FreeAgentOptions, LeagueId, PlayerId, ScoringPeriod, Season, SlotId,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{path, query_param},
};

fn client(server: &MockServer) -> Client {
    Client::builder()
        .base_url(format!("{}/apis/v3/games/", server.uri()))
        .build()
        .unwrap()
}

async fn load(server: &MockServer) -> espn_fantasy_football::LeagueHandle {
    let mut payload: Value =
        serde_json::from_str(include_str!("fixtures/football_2018_league.json")).unwrap();
    // Synthetic modern metadata; not a captured current ESPN response.
    payload["seasonId"] = json!(2024);
    payload["scoringPeriodId"] = json!(7);
    Mock::given(query_param("view", "mTeam"))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(server)
        .await;
    let mut league = client(server)
        .league(LeagueId(368876), Season(2024))
        .unwrap();
    league.fetch().await.unwrap();
    server.reset().await;
    league
}

fn player(id: i64, name: &str, week: u32) -> Value {
    json!({"id":id, "onTeamId":0, "status":"FREEAGENT", "transactions":[], "player":{
        "id":id, "fullName":name, "eligibleSlots":[0,7,20], "defaultPositionId":1,
        "proTeamId":16, "injuryStatus":"ACTIVE", "ownership":{"percentOwned":95.555},
        "stats":[{"seasonId":2024,"scoringPeriodId":week,"statSourceId":1,"appliedTotal":20.25}]
    }})
}

async fn aux(server: &MockServer) {
    Mock::given(query_param("view", "proTeamSchedules_wl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"settings":{"proTeams":[]}})))
        .mount(server)
        .await;
    Mock::given(query_param("view", "mPositionalRatings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server)
        .await;
}

#[tokio::test]
async fn invalid_queries_and_empty_id_inputs_make_no_requests() {
    let server = MockServer::start().await;
    let mut league = client(&server).league(LeagueId(1), Season(2024)).unwrap();
    assert!(league.players_by_ids(&[]).await.unwrap().is_empty());
    assert!(matches!(
        league.player_by_id(PlayerId(0)).await,
        Err(Error::Configuration(_))
    ));
    assert!(matches!(
        league.players_named(" ").await,
        Err(Error::Configuration(_))
    ));
    assert!(
        league
            .free_agents(FreeAgentOptions::default())
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    let mut league = load(&server).await;
    for options in [
        FreeAgentOptions {
            limit: 0,
            ..Default::default()
        },
        FreeAgentOptions {
            week: Some(ScoringPeriod(0)),
            ..Default::default()
        },
        FreeAgentOptions {
            offset: u32::MAX,
            ..Default::default()
        },
    ] {
        assert!(league.free_agents(options).await.is_err());
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn free_agent_page_preserves_qb_zero_future_week_and_snapshot() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    let before = serde_json::to_value(league.snapshot().unwrap()).unwrap();
    Mock::given(query_param("view", "kona_player_info"))
        .and(query_param("scoringPeriodId", "12"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"players":[player(1001,"One",12),player(1002,"Two",12)]})),
        )
        .expect(1)
        .mount(&server)
        .await;
    aux(&server).await;
    let page = league
        .free_agents(FreeAgentOptions {
            week: Some(ScoringPeriod(12)),
            limit: 2,
            offset: 4,
            slots: vec![SlotId(0)],
        })
        .await
        .unwrap();
    assert_eq!(page.scoring_period, ScoringPeriod(12));
    assert_eq!(page.next_offset, Some(6));
    assert_eq!(
        page.players
            .iter()
            .map(|player| player.player.id)
            .collect::<Vec<_>>(),
        [PlayerId(1001), PlayerId(1002)]
    );
    assert_eq!(page.players[0].projected_points, 20.25);
    assert_eq!(page.players[0].player.percent_owned, 95.56);
    let requests = server.received_requests().await.unwrap();
    let filter: Value =
        serde_json::from_slice(requests[0].headers["x-fantasy-filter"].as_bytes()).unwrap();
    assert_eq!(filter["players"]["filterSlotIds"]["value"], json!([0]));
    assert_eq!(filter["players"]["offset"], 4);
    assert_eq!(requests.len(), 3);
    assert_eq!(
        serde_json::to_value(league.snapshot().unwrap()).unwrap(),
        before
    );
}

#[tokio::test]
async fn pages_are_explicit_and_repeated_results_do_not_start_a_loop() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    Mock::given(query_param("view", "kona_player_info"))
        .respond_with(|request: &wiremock::Request| {
            let filter: Value =
                serde_json::from_slice(request.headers["x-fantasy-filter"].as_bytes()).unwrap();
            let offset = filter["players"]["offset"].as_u64().unwrap_or(0);
            let players = match offset {
                0 | 10 => vec![player(1001, "One", 7), player(1002, "Two", 7)],
                2 => vec![player(1003, "Three", 7)],
                _ => Vec::new(),
            };
            ResponseTemplate::new(200).set_body_json(json!({"players":players}))
        })
        .mount(&server)
        .await;
    aux(&server).await;
    for (offset, count, next) in [
        (0, 2, Some(2)),
        (2, 1, None),
        (3, 0, None),
        (10, 2, Some(12)),
    ] {
        let page = league
            .free_agents(FreeAgentOptions {
                limit: 2,
                offset,
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(page.players.len(), count);
        assert_eq!(page.next_offset, next);
    }
    assert_eq!(server.received_requests().await.unwrap().len(), 12);
}

#[tokio::test]
async fn directory_can_load_without_snapshot_and_names_return_all_ids() {
    let server = MockServer::start().await;
    Mock::given(path("/apis/v3/games/ffl/seasons/2024/players"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":1001,"fullName":"Same Name"},{"id":1002,"fullName":"Same Name"},{"id":-16016,"fullName":"Defense"}
        ]))).mount(&server).await;
    let mut league = client(&server)
        .league(LeagueId(368876), Season(2024))
        .unwrap();
    let directory = league.player_directory().await.unwrap();
    assert_eq!(
        directory.ids_named("Same Name"),
        [PlayerId(1001), PlayerId(1002)]
    );
    assert!(directory.ids_named("same name").is_empty());
    assert!(league.snapshot().is_none());
    league = load(&server).await;
    Mock::given(query_param("view", "players_wl"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id":1001,"fullName":"Same Name"},{"id":1002,"fullName":"Same Name"}
        ])))
        .mount(&server)
        .await;
    assert!(league.players_named("Unknown").await.unwrap().is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    Mock::given(query_param("view", "kona_playercard"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"players":[player(1002,"Same Name",7),player(1001,"Same Name",7)]}),
        ))
        .mount(&server)
        .await;
    aux(&server).await;
    let cards = league.players_named("Same Name").await.unwrap();
    assert_eq!(
        cards.iter().map(|card| card.player.id).collect::<Vec<_>>(),
        [PlayerId(1002), PlayerId(1001)]
    );
}

#[tokio::test]
async fn card_empty_unexpected_and_invalid_responses_preserve_snapshot() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    let before = serde_json::to_value(league.snapshot().unwrap()).unwrap();
    for response in [
        json!({"players":[]}),
        json!({"players":[player(999,"Wrong player",7)]}),
        json!({"players":null}),
    ] {
        server.reset().await;
        Mock::given(query_param("view", "kona_playercard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(response.clone()))
            .mount(&server)
            .await;
        aux(&server).await;
        let result = league.player_by_id(PlayerId(1001)).await;
        if response["players"] == json!([]) {
            assert!(result.unwrap().is_none());
        } else {
            assert!(result.is_err());
        }
        assert_eq!(
            serde_json::to_value(league.snapshot().unwrap()).unwrap(),
            before
        );
    }
}

#[tokio::test]
async fn failed_free_agent_auxiliary_reads_do_not_replace_snapshot() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    let before = serde_json::to_value(league.snapshot().unwrap()).unwrap();
    Mock::given(query_param("view", "kona_player_info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"players":[]})))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(query_param("view", "proTeamSchedules_wl"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        league
            .free_agents(FreeAgentOptions::default())
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(
        serde_json::to_value(league.snapshot().unwrap()).unwrap(),
        before
    );
}
