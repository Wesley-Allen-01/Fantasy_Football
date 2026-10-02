use espn_fantasy_football::{Client, LeagueId, MatchupPeriod, Season};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[tokio::test]
async fn scoreboard_selects_initial_endpoint_by_season() {
    for year in [2017, 2018, 2024] {
        let server = MockServer::start().await;
        let expected_path = if year < 2018 {
            "/apis/v3/games/ffl/leagueHistory/123".to_owned()
        } else {
            format!("/apis/v3/games/ffl/seasons/{year}/segments/0/leagues/123")
        };
        let response = json!({"schedule": []});
        Mock::given(method("GET"))
            .and(path(expected_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(if year < 2018 {
                json!([response])
            } else {
                response
            }))
            .expect(1)
            .mount(&server)
            .await;
        let client = Client::builder()
            .base_url(format!("{}/apis/v3/games/", server.uri()))
            .build()
            .unwrap();
        let mut league = client.league(LeagueId(123), Season(year)).unwrap();
        assert!(
            league
                .scoreboard(Some(MatchupPeriod(2)))
                .await
                .unwrap()
                .is_empty()
        );
        let requests = server.received_requests().await.unwrap();
        let query: Vec<_> = requests[0].url.query_pairs().collect();
        assert!(
            query
                .iter()
                .any(|(key, value)| key == "view" && value == "mMatchupScore")
        );
        assert!(!query.iter().any(|(key, _)| key == "scoringPeriodId"));
        assert_eq!(
            query
                .iter()
                .find(|(key, _)| key == "seasonId")
                .map(|(_, value)| value.as_ref()),
            if year < 2018 { Some("2017") } else { None }
        );
    }
}

#[tokio::test]
async fn historical_fallback_to_season_is_reused() {
    let server = MockServer::start().await;
    Mock::given(path("/apis/v3/games/ffl/leagueHistory/123"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path(
        "/apis/v3/games/ffl/seasons/2017/segments/0/leagues/123",
    ))
    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"schedule": []})))
    .expect(2)
    .mount(&server)
    .await;
    let client = Client::builder()
        .base_url(format!("{}/apis/v3/games/", server.uri()))
        .build()
        .unwrap();
    let mut league = client.league(LeagueId(123), Season(2017)).unwrap();
    for _ in 0..2 {
        assert!(
            league
                .scoreboard(Some(MatchupPeriod(2)))
                .await
                .unwrap()
                .is_empty()
        );
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    for request in &requests[1..] {
        assert!(!request.url.query_pairs().any(|(key, _)| key == "seasonId"));
    }
}
