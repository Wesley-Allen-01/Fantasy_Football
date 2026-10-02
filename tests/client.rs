use espn_fantasy_football::{Client, Credentials, Error, LeagueId, MatchupPeriod, Season};
use serde_json::{Value, json};
use std::time::Duration;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, query_param},
};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/football_2018_league.json")).unwrap()
}

fn client(server: &MockServer) -> Client {
    Client::builder()
        .base_url(format!("{}/apis/v3/games/", server.uri()))
        .build()
        .unwrap()
}

#[test]
fn configuration_and_debug_do_not_expose_private_cookies() {
    let credentials = Credentials::new("PRIVATE-S2", "{PRIVATE-SWID}").unwrap();
    let builder = Client::builder().credentials(credentials.clone());
    let builder_debug = format!("{builder:?}");
    let debug = format!(
        "{credentials:?} {builder_debug} {:?}",
        builder.build().unwrap()
    );
    assert!(!debug.contains("PRIVATE-S2"));
    assert!(!debug.contains("PRIVATE-SWID"));
    for (s2, swid) in [
        ("", "swid"),
        ("s2", ""),
        ("s2;other=cookie", "swid"),
        ("s2", "swid\r\n"),
    ] {
        assert!(Credentials::new(s2, swid).is_err());
    }
    assert!(Client::builder().timeout(Duration::ZERO).build().is_err());
    for url in [
        "ftp://example.com/apis/v3/games/",
        "https://example.com/",
        "https://user:pass@example.com/apis/v3/games/",
        "https://example.com/apis/v3/games/?query=1",
    ] {
        assert!(Client::builder().base_url(url).build().is_err());
    }
}

#[tokio::test]
async fn construction_is_lazy_and_default_scoreboard_requires_a_snapshot() {
    let server = MockServer::start().await;
    let client = client(&server);
    let mut league = client.league(LeagueId(368876), Season(2018)).unwrap();
    assert!(league.snapshot().is_none());
    assert!(matches!(
        league.scoreboard(None).await,
        Err(Error::Configuration(_))
    ));
    assert!(matches!(
        league.scoreboard(Some(MatchupPeriod(0))).await,
        Err(Error::Configuration(_))
    ));
    assert!(client.league(LeagueId(0), Season(2018)).is_err());
    assert!(client.league(LeagueId(1), Season(0)).is_err());
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn league_to_scoreboard_uses_reference_views_and_loaded_default() {
    let server = MockServer::start().await;
    let league_path = "/apis/v3/games/ffl/seasons/2018/segments/0/leagues/368876";
    Mock::given(method("GET"))
        .and(path(league_path))
        .and(query_param("view", "mTeam"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .expect(1)
        .mount(&server)
        .await;
    let scoreboard: Value =
        serde_json::from_str(include_str!("fixtures/football_2018_scoreboard.json")).unwrap();
    Mock::given(method("GET"))
        .and(path(league_path))
        .and(query_param("view", "mMatchupScore"))
        .respond_with(ResponseTemplate::new(200).set_body_json(scoreboard))
        .expect(1)
        .mount(&server)
        .await;
    let mut league = client(&server)
        .league(LeagueId(368876), Season(2018))
        .unwrap();
    let loaded = league.fetch().await.unwrap();
    assert_eq!(loaded.teams.len(), 10);
    assert_eq!(loaded.current_week.0, 16);
    let actual = league.scoreboard(None).await.unwrap();
    assert_eq!(actual.len(), 5);
    assert!(
        actual
            .iter()
            .all(|matchup| matchup.period == MatchupPeriod(16))
    );
    assert_eq!(actual[0].home_team, Some(espn_fantasy_football::TeamId(5)));
    assert_eq!(actual[0].away_team, Some(espn_fantasy_football::TeamId(2)));
    assert_eq!(actual[0].home_score, Some(174.56));
    assert_eq!(actual[0].away_score, Some(161.12));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let views: Vec<_> = requests[0]
        .url
        .query_pairs()
        .filter(|(k, _)| k == "view")
        .map(|(_, v)| v.into_owned())
        .collect();
    assert_eq!(
        views,
        ["mTeam", "mRoster", "mMatchup", "mSettings", "mStandings"]
    );
    let scoreboard_pairs: Vec<_> = requests[1]
        .url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    assert_eq!(
        scoreboard_pairs,
        vec![("view".into(), "mMatchupScore".into())]
    );
}

#[tokio::test]
async fn preseason_zero_current_week_can_use_the_default_scoreboard() {
    let server = MockServer::start().await;
    let mut payload = fixture();
    payload["scoringPeriodId"] = json!(0);
    Mock::given(query_param("view", "mTeam"))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(query_param("view", "mMatchupScore"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"schedule": []})))
        .expect(1)
        .mount(&server)
        .await;
    let mut league = client(&server)
        .league(LeagueId(368876), Season(2018))
        .unwrap();
    assert_eq!(league.fetch().await.unwrap().current_week.0, 0);
    assert!(league.scoreboard(None).await.unwrap().is_empty());
}

#[tokio::test]
async fn failed_http_and_model_refreshes_leave_snapshot_intact() {
    let server = MockServer::start().await;
    let mut league = client(&server)
        .league(LeagueId(368876), Season(2018))
        .unwrap();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .expect(1)
        .mount(&server)
        .await;
    let snapshot = league.fetch().await.unwrap();
    let original = serde_json::to_value(&snapshot).unwrap();
    server.reset().await;
    for response in [
        ResponseTemplate::new(503),
        ResponseTemplate::new(200).set_body_json(json!({"teams": []})),
    ] {
        Mock::given(method("GET"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        assert!(league.refresh().await.is_err());
        assert_eq!(
            serde_json::to_value(league.snapshot().unwrap()).unwrap(),
            original
        );
        server.reset().await;
    }
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(fixture()))
        .expect(2)
        .mount(&server)
        .await;
    for _ in 0..2 {
        let refreshed = league.refresh().await.unwrap();
        assert_eq!(serde_json::to_value(refreshed).unwrap(), original);
    }
}

#[tokio::test]
async fn redirects_are_reported_without_forwarding_private_cookies() {
    let primary = MockServer::start().await;
    let destination = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", destination.uri()))
        .expect(1)
        .mount(&primary)
        .await;
    let client = Client::builder()
        .base_url(format!("{}/apis/v3/games/", primary.uri()))
        .credentials(Credentials::new("s2", "swid").unwrap())
        .build()
        .unwrap();
    let mut league = client.league(LeagueId(1), Season(2024)).unwrap();
    assert!(matches!(
        league.scoreboard(Some(MatchupPeriod(1))).await,
        Err(Error::Http { status: 302 })
    ));
    assert!(destination.received_requests().await.unwrap().is_empty());
}
