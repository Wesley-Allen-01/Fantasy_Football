use crate::transport::EspnTransport;
use crate::{Error, LeagueSnapshot, MatchupPeriod, Result, ScoringPeriod};
use serde_json::{Value, json};

/// Resolved periods are separate: playoff matchups can span multiple NFL weeks.
pub(crate) struct WeeklyRequest {
    pub(crate) scoring_period: ScoringPeriod,
    pub(crate) matchup_period: MatchupPeriod,
    // Python retains a string settings-map key for an explicitly mapped week,
    // but uses the numeric status period for default/future/unmapped weeks.
    string_matchup_filter: bool,
}

pub(crate) struct WeeklyPayloads {
    pub(crate) box_scores: Value,
    pub(crate) pro_schedule: Value,
    pub(crate) positional_ratings: Value,
}

impl WeeklyRequest {
    pub(crate) fn resolve(snapshot: &LeagueSnapshot, week: Option<ScoringPeriod>) -> Result<Self> {
        if snapshot.season.0 < 2019 {
            return Err(Error::Configuration(
                "football box scores require season 2019 or later".into(),
            ));
        }
        if week == Some(ScoringPeriod(0)) {
            return Err(Error::Configuration(
                "box-score week must be greater than zero".into(),
            ));
        }
        let mut request = Self {
            scoring_period: snapshot.current_week,
            matchup_period: snapshot.current_matchup_period,
            string_matchup_filter: false,
        };
        if let Some(week) = week.filter(|week| *week <= snapshot.current_week) {
            request.scoring_period = week;
            if let Some((matchup, _)) = snapshot
                .settings
                .matchup_periods
                .iter()
                .find(|(_, weeks)| weeks.contains(&week))
            {
                request.matchup_period = *matchup;
                request.string_matchup_filter = true;
            }
        }
        Ok(request)
    }

    pub(crate) async fn fetch(&self, transport: &mut EspnTransport) -> Result<WeeklyPayloads> {
        let matchup_filter = if self.string_matchup_filter {
            json!(self.matchup_period.to_string())
        } else {
            json!(self.matchup_period)
        };
        let filter = json!({"schedule": {"filterMatchupPeriodIds": {"value": [matchup_filter]}}});
        let box_scores = transport
            .league_get(
                &["mMatchupScore", "mScoreboard"],
                Some(self.scoring_period),
                Some(&filter),
                "",
            )
            .await?;
        // Preserve Python's request ordering. A failing auxiliary read stops
        // the operation rather than returning a partially enriched matchup.
        let pro_schedule = transport
            .season_get(&["proTeamSchedules_wl"], None, None)
            .await?;
        let positional_ratings = transport
            .league_get(&["mPositionalRatings"], Some(self.scoring_period), None, "")
            .await?;
        Ok(WeeklyPayloads {
            box_scores,
            pro_schedule,
            positional_ratings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Credentials, LeagueId, Season};
    use std::time::Duration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    const LEAGUE_PATH: &str = "/apis/v3/games/ffl/seasons/2024/segments/0/leagues/123";
    const SEASON_PATH: &str = "/apis/v3/games/ffl/seasons/2024";
    const HISTORY_PATH: &str = "/apis/v3/games/ffl/leagueHistory/123";

    fn transport(server: &MockServer) -> EspnTransport {
        EspnTransport::new(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            reqwest::Url::parse(&format!("{}/apis/v3/games/", server.uri())).unwrap(),
            LeagueId(123),
            Season(2024),
            Some(Credentials::new("s2", "swid").unwrap()),
        )
        .unwrap()
    }

    fn snapshot() -> LeagueSnapshot {
        let value =
            serde_json::from_str(include_str!("../tests/fixtures/football_2018_league.json"))
                .unwrap();
        let mut snapshot =
            LeagueSnapshot::from_value(&value, LeagueId(368876), Season(2018)).unwrap();
        // Period selection is pure; the historical fixture supplies unrelated
        // settings while these fields exercise synthetic multi-week matchups.
        snapshot.season = Season(2024);
        snapshot.current_week = ScoringPeriod(6);
        snapshot.current_matchup_period = MatchupPeriod(99);
        snapshot.settings.matchup_periods = [
            (MatchupPeriod(1), vec![ScoringPeriod(1), ScoringPeriod(2)]),
            (MatchupPeriod(2), vec![ScoringPeriod(3), ScoringPeriod(4)]),
            (MatchupPeriod(3), vec![ScoringPeriod(5), ScoringPeriod(6)]),
        ]
        .into();
        snapshot
    }

    #[test]
    fn resolves_default_explicit_current_past_future_and_unmapped_weeks() {
        let mut snapshot = snapshot();
        for (week, expected_scoring, expected_matchup) in [
            (None, 6, 99),
            (Some(2), 2, 1),
            (Some(6), 6, 3),
            (Some(100), 6, 99),
        ] {
            let request = WeeklyRequest::resolve(&snapshot, week.map(ScoringPeriod)).unwrap();
            assert_eq!(request.scoring_period, ScoringPeriod(expected_scoring));
            assert_eq!(request.matchup_period, MatchupPeriod(expected_matchup));
            assert_eq!(request.string_matchup_filter, matches!(week, Some(2 | 6)));
        }
        snapshot.settings.matchup_periods.remove(&MatchupPeriod(2));
        let request = WeeklyRequest::resolve(&snapshot, Some(ScoringPeriod(4))).unwrap();
        assert_eq!(request.scoring_period, ScoringPeriod(4));
        assert_eq!(request.matchup_period, MatchupPeriod(99));
        assert!(!request.string_matchup_filter);
    }

    #[test]
    fn rejects_pre_2019_season_and_explicit_zero() {
        let mut snapshot = snapshot();
        assert!(matches!(
            WeeklyRequest::resolve(&snapshot, Some(ScoringPeriod(0))),
            Err(Error::Configuration(_))
        ));
        snapshot.season = Season(2018);
        assert!(matches!(
            WeeklyRequest::resolve(&snapshot, None),
            Err(Error::Configuration(_))
        ));
        snapshot.season = Season(2019);
        assert!(WeeklyRequest::resolve(&snapshot, None).is_ok());
    }

    #[tokio::test]
    async fn fetches_exact_views_filters_periods_and_auxiliary_order() {
        let server = MockServer::start().await;
        let box_scores = json!({"schedule": []});
        let pro_schedule = json!({"settings": {"proTeams": []}});
        let positional_ratings = json!({"positionAgainstOpponent": {"positionalRatings": {}}});
        Mock::given(method("GET"))
            .and(path(LEAGUE_PATH))
            .and(query_param("view", "mMatchupScore"))
            .and(query_param("view", "mScoreboard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&box_scores))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(SEASON_PATH))
            .and(query_param("view", "proTeamSchedules_wl"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&pro_schedule))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(LEAGUE_PATH))
            .and(query_param("view", "mPositionalRatings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&positional_ratings))
            .expect(1)
            .mount(&server)
            .await;
        let request = WeeklyRequest {
            scoring_period: ScoringPeriod(5),
            matchup_period: MatchupPeriod(3),
            string_matchup_filter: true,
        };
        let payloads = request.fetch(&mut transport(&server)).await.unwrap();
        assert_eq!(payloads.box_scores, box_scores);
        assert_eq!(payloads.pro_schedule, pro_schedule);
        assert_eq!(payloads.positional_ratings, positional_ratings);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(
            requests
                .iter()
                .map(|request| request.url.path())
                .collect::<Vec<_>>(),
            vec![LEAGUE_PATH, SEASON_PATH, LEAGUE_PATH]
        );
        for (index, request) in requests.iter().enumerate() {
            let query: Vec<_> = request.url.query_pairs().collect();
            assert!(!query.iter().any(|(key, _)| key == "seasonId"));
            assert_eq!(
                query
                    .iter()
                    .find(|(key, _)| key == "scoringPeriodId")
                    .map(|(_, value)| value.as_ref()),
                if index == 1 { None } else { Some("5") }
            );
            assert_eq!(request.headers["cookie"], "espn_s2=s2; SWID=swid");
            if index == 0 {
                assert_eq!(
                    query
                        .iter()
                        .filter(|(key, _)| key == "view")
                        .map(|(_, value)| value.as_ref())
                        .collect::<Vec<_>>(),
                    vec!["mMatchupScore", "mScoreboard"]
                );
                assert_eq!(
                    serde_json::from_slice::<Value>(request.headers["x-fantasy-filter"].as_bytes())
                        .unwrap(),
                    json!({"schedule": {"filterMatchupPeriodIds": {"value": ["3"]}}})
                );
            } else {
                assert!(!request.headers.contains_key("x-fantasy-filter"));
            }
        }
    }

    #[tokio::test]
    async fn stops_on_primary_and_auxiliary_errors_without_extra_requests() {
        for (box_status, pro_status, rating_status, expected_requests) in
            [(500, 200, 200, 1), (200, 503, 200, 2), (200, 200, 403, 3)]
        {
            let server = MockServer::start().await;
            Mock::given(path(LEAGUE_PATH))
                .and(query_param("view", "mScoreboard"))
                .respond_with(
                    ResponseTemplate::new(box_status).set_body_json(json!({"schedule": []})),
                )
                .mount(&server)
                .await;
            Mock::given(path(SEASON_PATH))
                .respond_with(
                    ResponseTemplate::new(pro_status)
                        .set_body_json(json!({"settings": {"proTeams": []}})),
                )
                .mount(&server)
                .await;
            Mock::given(path(LEAGUE_PATH))
                .and(query_param("view", "mPositionalRatings"))
                .respond_with(ResponseTemplate::new(rating_status).set_body_json(json!({})))
                .mount(&server)
                .await;
            let request = WeeklyRequest {
                scoring_period: ScoringPeriod(5),
                matchup_period: MatchupPeriod(3),
                string_matchup_filter: true,
            };
            assert!(matches!(
                request.fetch(&mut transport(&server)).await,
                Err(Error::Http { .. })
            ));
            assert_eq!(
                server.received_requests().await.unwrap().len(),
                expected_requests
            );
        }
    }

    #[tokio::test]
    async fn league_fallback_is_reused_by_ratings_but_not_season_schedule() {
        let server = MockServer::start().await;
        Mock::given(path(LEAGUE_PATH))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .and(query_param("view", "mScoreboard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"schedule": []}])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(SEASON_PATH))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"settings": {"proTeams": []}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .and(query_param("view", "mPositionalRatings"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!([{"positionAgainstOpponent": {}}])),
            )
            .expect(1)
            .mount(&server)
            .await;
        let request = WeeklyRequest {
            scoring_period: ScoringPeriod(5),
            matchup_period: MatchupPeriod(3),
            string_matchup_filter: true,
        };
        request.fetch(&mut transport(&server)).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.url.path())
                .collect::<Vec<_>>(),
            vec![LEAGUE_PATH, HISTORY_PATH, SEASON_PATH, HISTORY_PATH]
        );
        for index in [1, 3] {
            assert!(
                requests[index]
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "seasonId" && value == "2024")
            );
            assert!(
                requests[index]
                    .url
                    .query_pairs()
                    .any(|(key, value)| key == "scoringPeriodId" && value == "5")
            );
        }
        assert!(
            !requests[2]
                .url
                .query_pairs()
                .any(|(key, _)| key == "seasonId" || key == "scoringPeriodId")
        );
    }

    #[tokio::test]
    async fn ordered_request_traces_match_the_closed_python_oracle() {
        let oracle: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/weekly/python_requests.json"
        ))
        .unwrap();
        let mut snapshot = snapshot();
        snapshot.current_week = ScoringPeriod(8);
        snapshot.current_matchup_period = MatchupPeriod(4);
        snapshot.settings.matchup_periods =
            [(MatchupPeriod(4), vec![ScoringPeriod(7), ScoringPeriod(8)])].into();
        for case in oracle["cases"].as_array().unwrap() {
            let server = MockServer::start().await;
            Mock::given(path(LEAGUE_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .expect(2)
                .mount(&server)
                .await;
            Mock::given(path(SEASON_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .expect(1)
                .mount(&server)
                .await;
            let week = case["requested_week"]
                .as_u64()
                .map(|week| ScoringPeriod(u32::try_from(week).unwrap()));
            let request = WeeklyRequest::resolve(&snapshot, week).unwrap();
            assert_eq!(
                json!(request.scoring_period),
                case["resolved_scoring_period"]
            );
            assert_eq!(
                json!(request.matchup_period),
                case["resolved_matchup_period"]
            );
            request.fetch(&mut transport(&server)).await.unwrap();
            let requests = server.received_requests().await.unwrap();
            let trace: Vec<Value> = requests.iter().map(|request| {
                let mut query = serde_json::Map::new();
                for (key, value) in request.url.query_pairs() {
                    query.entry(key.into_owned()).or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!(value));
                }
                let filter = request.headers.get("x-fantasy-filter").map(|header| serde_json::from_slice::<Value>(header.as_bytes()).unwrap()).unwrap_or(Value::Null);
                json!({"method": request.method.as_str(), "path": request.url.path(), "query": query, "fantasy_filter": filter})
            }).collect();
            assert_eq!(json!(trace), case["requests"], "requested week {week:?}");
        }
    }
}
