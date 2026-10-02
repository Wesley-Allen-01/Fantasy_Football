use espn_fantasy_football::{
    Client, Credentials, Error, LeagueId, MatchupPeriod, PlayerId, PlayerTeamHistory, ProTeamId,
    ScoringPeriod, Season, TeamId,
};
use serde_json::{Value, json};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

fn league_payload() -> Value {
    let mut value: Value =
        serde_json::from_str(include_str!("fixtures/football_2018_league.json")).unwrap();
    // Synthetic modern league metadata, not a captured current-season response.
    value["seasonId"] = json!(2023);
    value["scoringPeriodId"] = json!(7);
    value["status"]["currentMatchupPeriod"] = json!(4);
    value["settings"]["scheduleSettings"]["matchupPeriods"] =
        json!({"1":[1,2], "2":[3,4], "3":[5,6], "4":[7,8]});
    value
}

fn box_payload() -> Value {
    json!({"schedule":[{
        "id":41, "matchupPeriodId":4,
        "home": {
            "teamId":1, "totalPoints":10.0, "totalPointsLive":100.125,
            "totalProjectedPointsLive":110.5,
            "rosterForCurrentScoringPeriod":{"entries":[{
                "playerId":1001, "lineupSlotId":4,
                "playerPoolEntry":{"id":1001, "onTeamId":1, "player":{
                    "id":1001, "fullName":"Example receiver", "eligibleSlots":[4,20],
                    "defaultPositionId":4, "proTeamId":16, "injuryStatus":"ACTIVE",
                    "stats":[{
                        "seasonId":2023, "scoringPeriodId":7, "statSourceId":0,
                        "proTeamId":11, "appliedTotal":18.567, "stats":{"42":100},
                        "appliedStats":{"42":18.567}
                    },{
                        "seasonId":2023, "scoringPeriodId":7, "statSourceId":1,
                        "proTeamId":16, "appliedTotal":20.25
                    }]
                }}
            }]}
        }
    }]})
}

fn schedule_payload() -> Value {
    json!({"settings":{"proTeams":[{
        "id":11, "proGamesByScoringPeriod":{"7":[{
            "homeProTeamId":11, "awayProTeamId":12, "date":1700000000000_i64
        }]}
    }]}})
}

fn ratings_payload() -> Value {
    json!({"positionAgainstOpponent":{"positionalRatings":{
        "4":{"ratingsByOpponent":{"12":{"rank":3}}}
    }}})
}

fn client(server: &MockServer) -> Client {
    Client::builder()
        .base_url(format!("{}/apis/v3/games/", server.uri()))
        .credentials(Credentials::new("test-s2", "{test-swid}").unwrap())
        .build()
        .unwrap()
}

async fn load(server: &MockServer) -> espn_fantasy_football::LeagueHandle {
    Mock::given(query_param("view", "mTeam"))
        .respond_with(ResponseTemplate::new(200).set_body_json(league_payload()))
        .expect(1)
        .mount(server)
        .await;
    let mut league = client(server)
        .league(LeagueId(368876), Season(2023))
        .unwrap();
    league.fetch().await.unwrap();
    league
}

async fn weekly_mocks(server: &MockServer, responses: [ResponseTemplate; 3]) {
    for ((view, endpoint), response) in [
        (
            "mScoreboard",
            "/apis/v3/games/ffl/seasons/2023/segments/0/leagues/368876",
        ),
        ("proTeamSchedules_wl", "/apis/v3/games/ffl/seasons/2023"),
        (
            "mPositionalRatings",
            "/apis/v3/games/ffl/seasons/2023/segments/0/leagues/368876",
        ),
    ]
    .into_iter()
    .zip(responses)
    {
        Mock::given(method("GET"))
            .and(path(endpoint))
            .and(query_param("view", view))
            .respond_with(response)
            .mount(server)
            .await;
    }
}

fn successful_responses() -> [ResponseTemplate; 3] {
    [
        ResponseTemplate::new(200).set_body_json(box_payload()),
        ResponseTemplate::new(200).set_body_json(schedule_payload()),
        ResponseTemplate::new(200).set_body_json(ratings_payload()),
    ]
}

#[tokio::test]
async fn configuration_failures_do_not_issue_weekly_requests() {
    let server = MockServer::start().await;
    let client = client(&server);
    let mut historical = client.league(LeagueId(368876), Season(2018)).unwrap();
    assert!(matches!(
        historical.box_scores(None).await,
        Err(Error::Configuration(_))
    ));
    let mut unloaded = client.league(LeagueId(368876), Season(2023)).unwrap();
    assert!(matches!(
        unloaded.box_scores(None).await,
        Err(Error::Configuration(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
    let mut league = load(&server).await;
    server.reset().await;
    assert!(matches!(
        league.box_scores(Some(ScoringPeriod(0))).await,
        Err(Error::Configuration(_))
    ));
    let mut wrong_year = PlayerTeamHistory::new(Season(2022));
    assert!(matches!(
        league.box_scores_with_history(None, &mut wrong_year).await,
        Err(Error::Configuration(_))
    ));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn weekly_requests_return_one_team_matchup_without_replacing_roster() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    let snapshot_before = serde_json::to_value(league.snapshot().unwrap()).unwrap();
    server.reset().await;
    weekly_mocks(&server, successful_responses()).await;
    let mut history = PlayerTeamHistory::new(Season(2023));
    let weekly = league
        .box_scores_with_history(Some(ScoringPeriod(7)), &mut history)
        .await
        .unwrap();
    assert_eq!(weekly.scoring_period, ScoringPeriod(7));
    assert_eq!(weekly.matchup_period, MatchupPeriod(4));
    let matchup = weekly.for_team(TeamId(1)).unwrap();
    assert!(weekly.for_team(TeamId(999)).is_none());
    assert!(matchup.away.is_none());
    let home = matchup.home.as_ref().unwrap();
    assert_eq!(home.score, 100.12);
    assert_eq!(home.projected, 110.5);
    let player = &home.lineup[0];
    assert_eq!(player.points, 18.57);
    assert_eq!(player.projected_points, 20.25);
    assert_eq!(player.pro_team, ProTeamId(11));
    assert_eq!(player.pro_opponent, Some(ProTeamId(12)));
    assert_eq!(player.pro_pos_rank, Some(3));
    assert_eq!(history.get(PlayerId(1001)), Some(ProTeamId(11)));
    assert_eq!(
        serde_json::to_value(league.snapshot().unwrap()).unwrap(),
        snapshot_before
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let views: Vec<_> = requests[0]
        .url
        .query_pairs()
        .filter(|(k, _)| k == "view")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(views, ["mMatchupScore", "mScoreboard"]);
    assert_eq!(
        requests[0]
            .url
            .query_pairs()
            .find(|(k, _)| k == "scoringPeriodId")
            .unwrap()
            .1,
        "7"
    );
    assert_eq!(
        serde_json::from_str::<Value>(requests[0].headers["x-fantasy-filter"].to_str().unwrap())
            .unwrap(),
        json!({"schedule":{"filterMatchupPeriodIds":{"value":["4"]}}})
    );
    assert!(
        !requests[1]
            .url
            .query_pairs()
            .any(|(key, _)| key == "scoringPeriodId")
    );
    assert!(
        requests
            .iter()
            .all(|request| request.headers["cookie"] == "espn_s2=test-s2; SWID={test-swid}")
    );
}

#[tokio::test]
async fn default_and_future_week_use_loaded_current_periods() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    server.reset().await;
    weekly_mocks(&server, successful_responses()).await;
    for week in [None, Some(ScoringPeriod(30))] {
        let result = league.box_scores(week).await.unwrap();
        assert_eq!(result.scoring_period, ScoringPeriod(7));
        assert_eq!(result.matchup_period, MatchupPeriod(4));
    }
}

#[tokio::test]
async fn request_and_parse_failures_leave_history_and_snapshot_intact() {
    let server = MockServer::start().await;
    let mut league = load(&server).await;
    let snapshot_before = serde_json::to_value(league.snapshot().unwrap()).unwrap();
    let mut history = PlayerTeamHistory::new(Season(2023));
    history.insert(PlayerId(999), ProTeamId(16));
    let history_before = history.teams().clone();
    for index in 0..4 {
        server.reset().await;
        let mut responses = successful_responses();
        if index < 3 {
            responses[index] = ResponseTemplate::new(503);
        } else {
            let mut payload = box_payload();
            // First lineup contains actual evidence. A later malformed side
            // must prevent that evidence from being committed to history.
            payload["schedule"]
                .as_array_mut()
                .unwrap()
                .push(json!({"home":{"teamId":2}}));
            responses[0] = ResponseTemplate::new(200).set_body_json(payload);
        }
        weekly_mocks(&server, responses).await;
        assert!(
            league
                .box_scores_with_history(None, &mut history)
                .await
                .is_err()
        );
        assert_eq!(history.teams(), &history_before);
        assert_eq!(
            serde_json::to_value(league.snapshot().unwrap()).unwrap(),
            snapshot_before
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            if index < 3 { index + 1 } else { 3 }
        );
    }
}
