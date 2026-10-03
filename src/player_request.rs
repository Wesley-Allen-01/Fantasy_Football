use crate::football::FreeAgentOptions;
use crate::transport::EspnTransport;
use crate::{Error, LeagueSnapshot, PlayerId, Result, ScoringPeriod, Season};
use serde_json::{Value, json};
use std::collections::HashSet;

pub(crate) struct FreeAgentRequest {
    pub(crate) scoring_period: ScoringPeriod,
    filter: Value,
}

pub(crate) struct PlayerPayloads {
    pub(crate) players: Value,
    pub(crate) pro_schedule: Value,
    pub(crate) positional_ratings: Value,
}

pub(crate) struct CardPayloads {
    pub(crate) players: Value,
    pub(crate) pro_schedule: Value,
}

impl FreeAgentRequest {
    pub(crate) fn resolve(snapshot: &LeagueSnapshot, options: &FreeAgentOptions) -> Result<Self> {
        if snapshot.season.0 < 2019 {
            return Err(Error::Configuration(
                "football free agents require season 2019 or later".into(),
            ));
        }
        if options.week == Some(ScoringPeriod(0)) {
            return Err(Error::Configuration(
                "free-agent week must be greater than zero".into(),
            ));
        }
        if options.limit == 0 || options.offset.checked_add(options.limit).is_none() {
            return Err(Error::Configuration(
                "free-agent limit must be positive and offset plus limit must fit u32".into(),
            ));
        }
        let mut filter = json!({"players": {
            "filterStatus": {"value": ["FREEAGENT", "WAIVERS"]},
            "filterSlotIds": {"value": options.slots},
            "limit": options.limit,
            "sortPercOwned": {"sortPriority": 1, "sortAsc": false},
            "sortDraftRanks": {"sortPriority": 100, "sortAsc": true, "value": "STANDARD"},
        }});
        if options.offset != 0 {
            filter["players"]["offset"] = json!(options.offset);
        }
        Ok(Self {
            scoring_period: options.week.unwrap_or(snapshot.current_week),
            filter,
        })
    }

    pub(crate) async fn fetch(&self, transport: &mut EspnTransport) -> Result<PlayerPayloads> {
        let players = transport
            .league_get(
                &["kona_player_info"],
                Some(self.scoring_period),
                Some(&self.filter),
                "",
            )
            .await?;
        player_rows(&players)?;
        // An empty page still receives Python's two auxiliary reads. Pagination
        // is explicit: this request never fetches another offset automatically.
        let pro_schedule = transport
            .season_get(&["proTeamSchedules_wl"], None, None)
            .await?;
        let positional_ratings = transport
            .league_get(&["mPositionalRatings"], Some(self.scoring_period), None, "")
            .await?;
        Ok(PlayerPayloads {
            players,
            pro_schedule,
            positional_ratings,
        })
    }
}

pub(crate) async fn fetch_player_cards(
    transport: &mut EspnTransport,
    season: Season,
    ids: &[PlayerId],
    final_period: ScoringPeriod,
) -> Result<CardPayloads> {
    if ids.iter().any(|id| id.0 == 0) {
        return Err(Error::Configuration("player ID must be nonzero".into()));
    }
    if ids.is_empty() {
        return Ok(CardPayloads {
            players: json!({"players": []}),
            pro_schedule: json!({"settings": {"proTeams": []}}),
        });
    }
    let mut seen = HashSet::new();
    let ids: Vec<_> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    let mut players = Vec::new();
    for batch in ids.chunks(40) {
        let filter = json!({"players": {
            "filterIds": {"value": batch},
            "filterStatsForTopScoringPeriodIds": {
                "value": final_period,
                "additionalValue": [format!("00{season}"), format!("10{season}")],
            },
        }});
        let response = transport
            .league_get(&["kona_playercard"], None, Some(&filter), "")
            .await?;
        players.extend(player_rows(&response)?.iter().cloned());
    }
    let pro_schedule = transport
        .season_get(&["proTeamSchedules_wl"], None, None)
        .await?;
    Ok(CardPayloads {
        players: json!({"players": players}),
        pro_schedule,
    })
}

pub(crate) async fn fetch_player_directory(transport: &EspnTransport) -> Result<Value> {
    transport
        .season_resource_get(
            &["players_wl"],
            None,
            Some(&json!({"filterActive": {"value": true}})),
            "/players",
        )
        .await
}

fn player_rows(value: &Value) -> Result<&Vec<Value>> {
    let rows = value
        .get("players")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::InvalidResponse {
            context: "player response must contain a players array".into(),
        })?;
    if rows.iter().any(|row| !row.is_object()) {
        return Err(Error::InvalidResponse {
            context: "player response contains a non-object player wrapper".into(),
        });
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Credentials, LeagueId, SlotId};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{path, query_param},
    };

    const LEAGUE_PATH: &str = "/apis/v3/games/ffl/seasons/2024/segments/0/leagues/123";
    const SEASON_PATH: &str = "/apis/v3/games/ffl/seasons/2024";
    const HISTORY_PATH: &str = "/apis/v3/games/ffl/leagueHistory/123";

    fn transport(server: &MockServer, season: Season) -> EspnTransport {
        EspnTransport::new(
            reqwest::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            reqwest::Url::parse(&format!("{}/apis/v3/games/", server.uri())).unwrap(),
            LeagueId(123),
            season,
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
        snapshot.season = Season(2024);
        snapshot.current_week = ScoringPeriod(8);
        snapshot
    }

    async fn schedules(server: &MockServer) {
        Mock::given(path(SEASON_PATH))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"settings": {"proTeams": []}})),
            )
            .mount(server)
            .await;
    }

    fn filter(request: &wiremock::Request) -> Value {
        serde_json::from_slice(request.headers["x-fantasy-filter"].as_bytes()).unwrap()
    }

    #[test]
    fn resolves_defaults_future_week_slots_and_offset_without_clamping() {
        let snapshot = snapshot();
        let default = FreeAgentRequest::resolve(&snapshot, &FreeAgentOptions::default()).unwrap();
        assert_eq!(default.scoring_period, ScoringPeriod(8));
        assert_eq!(
            default.filter,
            json!({"players": {
                "filterStatus": {"value": ["FREEAGENT", "WAIVERS"]},
                "filterSlotIds": {"value": []}, "limit": 50,
                "sortPercOwned": {"sortPriority": 1, "sortAsc": false},
                "sortDraftRanks": {"sortPriority": 100, "sortAsc": true, "value": "STANDARD"},
            }})
        );
        let options = FreeAgentOptions {
            week: Some(ScoringPeriod(99)),
            limit: 2,
            offset: 50,
            slots: vec![SlotId(0), SlotId(999)],
        };
        let request = FreeAgentRequest::resolve(&snapshot, &options).unwrap();
        assert_eq!(request.scoring_period, ScoringPeriod(99));
        assert_eq!(
            request.filter["players"]["filterSlotIds"]["value"],
            json!([0, 999])
        );
        assert_eq!(request.filter["players"]["offset"], 50);
        assert_eq!(request.filter["players"]["limit"], 2);
    }

    #[test]
    fn rejects_historical_zero_and_overflow_options() {
        let mut snapshot = snapshot();
        for options in [
            FreeAgentOptions {
                week: Some(ScoringPeriod(0)),
                ..FreeAgentOptions::default()
            },
            FreeAgentOptions {
                limit: 0,
                ..FreeAgentOptions::default()
            },
            FreeAgentOptions {
                offset: u32::MAX,
                limit: 1,
                ..FreeAgentOptions::default()
            },
        ] {
            assert!(matches!(
                FreeAgentRequest::resolve(&snapshot, &options),
                Err(Error::Configuration(_))
            ));
        }
        snapshot.season = Season(2018);
        assert!(matches!(
            FreeAgentRequest::resolve(&snapshot, &FreeAgentOptions::default()),
            Err(Error::Configuration(_))
        ));
        snapshot.season = Season(2019);
        assert!(
            FreeAgentRequest::resolve(
                &snapshot,
                &FreeAgentOptions {
                    offset: u32::MAX - 1,
                    limit: 1,
                    ..FreeAgentOptions::default()
                }
            )
            .is_ok()
        );
    }

    #[tokio::test]
    async fn free_agent_empty_full_short_and_repeated_pages_are_explicit() {
        for rows in [json!([]), json!([{"id": 1}]), json!([{"id": 1}, {"id": 2}])] {
            let server = MockServer::start().await;
            Mock::given(path(LEAGUE_PATH))
                .and(query_param("view", "kona_player_info"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"players": rows})))
                .expect(2)
                .mount(&server)
                .await;
            schedules(&server).await;
            Mock::given(path(LEAGUE_PATH))
                .and(query_param("view", "mPositionalRatings"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(
                        json!({"positionAgainstOpponent": {"positionalRatings": {}}}),
                    ),
                )
                .expect(2)
                .mount(&server)
                .await;
            let options = FreeAgentOptions {
                week: Some(ScoringPeriod(99)),
                limit: 2,
                offset: 20,
                slots: vec![SlotId(0)],
            };
            let request = FreeAgentRequest::resolve(&snapshot(), &options).unwrap();
            let mut transport = transport(&server, Season(2024));
            for _ in 0..2 {
                let result = request.fetch(&mut transport).await.unwrap();
                assert_eq!(result.players["players"], rows);
                assert_eq!(result.pro_schedule, json!({"settings": {"proTeams": []}}));
                assert_eq!(
                    result.positional_ratings,
                    json!({"positionAgainstOpponent": {"positionalRatings": {}}})
                );
            }
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 6);
            for batch in requests.chunks(3) {
                assert_eq!(
                    batch
                        .iter()
                        .map(|request| request.url.path())
                        .collect::<Vec<_>>(),
                    vec![LEAGUE_PATH, SEASON_PATH, LEAGUE_PATH]
                );
                assert_eq!(filter(&batch[0]), request.filter);
                assert!(
                    batch[0]
                        .url
                        .query_pairs()
                        .any(|(key, value)| key == "scoringPeriodId" && value == "99")
                );
                assert!(
                    batch[2]
                        .url
                        .query_pairs()
                        .any(|(key, value)| key == "scoringPeriodId" && value == "99")
                );
                assert!(
                    !batch[1]
                        .url
                        .query_pairs()
                        .any(|(key, _)| key == "scoringPeriodId")
                );
                for request in batch {
                    assert_eq!(request.headers["cookie"], "espn_s2=s2; SWID=swid");
                }
            }
        }
    }

    #[tokio::test]
    async fn invalid_free_agent_players_stop_before_auxiliary_requests() {
        for body in [
            json!({}),
            json!({"players": null}),
            json!({"players": {}}),
            json!({"players": [1]}),
        ] {
            let server = MockServer::start().await;
            Mock::given(path(LEAGUE_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .expect(1)
                .mount(&server)
                .await;
            let request =
                FreeAgentRequest::resolve(&snapshot(), &FreeAgentOptions::default()).unwrap();
            assert!(matches!(
                request.fetch(&mut transport(&server, Season(2024))).await,
                Err(Error::InvalidResponse { .. })
            ));
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn card_batches_deduplicate_stably_and_preserve_each_server_order() {
        let server = MockServer::start().await;
        Mock::given(path(LEAGUE_PATH)).and(query_param("view", "kona_playercard"))
            .respond_with(|request: &wiremock::Request| {
                let ids = filter(request)["players"]["filterIds"]["value"].as_array().unwrap().clone();
                ResponseTemplate::new(200).set_body_json(json!({"players": ids.into_iter().rev().map(|id| json!({"player": {"id": id}})).collect::<Vec<_>>()}))
            }).expect(3).mount(&server).await;
        schedules(&server).await;
        let mut ids = vec![PlayerId(-16)];
        ids.extend((1..=80).map(PlayerId));
        let unique = ids.clone();
        ids.extend([PlayerId(1), PlayerId(-16)]);
        let result = fetch_player_cards(
            &mut transport(&server, Season(2024)),
            Season(2024),
            &ids,
            ScoringPeriod(18),
        )
        .await
        .unwrap();
        let expected: Vec<_> = unique
            .chunks(40)
            .flat_map(|batch| batch.iter().rev())
            .map(|id| json!({"player": {"id": id}}))
            .collect();
        assert_eq!(result.players["players"], json!(expected));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 4);
        for (index, batch) in unique.chunks(40).enumerate() {
            assert_eq!(
                filter(&requests[index]),
                json!({"players": {
                    "filterIds": {"value": batch},
                    "filterStatsForTopScoringPeriodIds": {"value": 18, "additionalValue": ["002024", "102024"]},
                }})
            );
            assert!(
                !requests[index]
                    .url
                    .query_pairs()
                    .any(|(key, _)| key == "scoringPeriodId")
            );
        }
        assert_eq!(requests[3].url.path(), SEASON_PATH);
    }

    #[tokio::test]
    async fn empty_and_invalid_card_ids_perform_no_io() {
        let server = MockServer::start().await;
        let mut transport = transport(&server, Season(2024));
        let empty = fetch_player_cards(&mut transport, Season(2024), &[], ScoringPeriod(18))
            .await
            .unwrap();
        assert_eq!(empty.players, json!({"players": []}));
        assert!(matches!(
            fetch_player_cards(
                &mut transport,
                Season(2024),
                &[PlayerId(1), PlayerId(0)],
                ScoringPeriod(18)
            )
            .await,
            Err(Error::Configuration(_))
        ));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn empty_server_card_result_still_fetches_schedule_for_historical_season() {
        let server = MockServer::start().await;
        Mock::given(path(HISTORY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"players": []}])))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/apis/v3/games/ffl/seasons/2015"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"settings": {"proTeams": []}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = fetch_player_cards(
            &mut transport(&server, Season(2015)),
            Season(2015),
            &[PlayerId(-16)],
            ScoringPeriod(17),
        )
        .await
        .unwrap();
        assert_eq!(result.players, json!({"players": []}));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            filter(&requests[0])["players"]["filterStatsForTopScoringPeriodIds"]["additionalValue"],
            json!(["002015", "102015"])
        );
        assert!(
            requests[0]
                .url
                .query_pairs()
                .any(|(key, value)| key == "seasonId" && value == "2015")
        );
    }

    #[tokio::test]
    async fn partial_card_batches_fail_without_schedule_or_result() {
        for second_response in [
            ResponseTemplate::new(503),
            ResponseTemplate::new(200).set_body_json(json!({})),
            ResponseTemplate::new(200).set_body_json(json!({"players": null})),
            ResponseTemplate::new(200).set_body_json(json!({"players": [1]})),
        ] {
            let server = MockServer::start().await;
            let counter = Arc::new(AtomicUsize::new(0));
            Mock::given(path(LEAGUE_PATH))
                .respond_with(move |_: &wiremock::Request| {
                    if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        ResponseTemplate::new(200)
                            .set_body_json(json!({"players": [{"player": {"id": 1}}]}))
                    } else {
                        second_response.clone()
                    }
                })
                .expect(2)
                .mount(&server)
                .await;
            let ids: Vec<_> = (1..=41).map(PlayerId).collect();
            assert!(
                fetch_player_cards(
                    &mut transport(&server, Season(2024)),
                    Season(2024),
                    &ids,
                    ScoringPeriod(18)
                )
                .await
                .is_err()
            );
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
        }
    }

    #[tokio::test]
    async fn directory_uses_season_players_resource_and_keeps_raw_array() {
        let server = MockServer::start().await;
        let directory =
            json!([{"id": 1, "fullName": "Duplicate"}, {"id": 2, "fullName": "Duplicate"}]);
        Mock::given(path(format!("{SEASON_PATH}/players")))
            .respond_with(ResponseTemplate::new(200).set_body_json(&directory))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            fetch_player_directory(&transport(&server, Season(2024)))
                .await
                .unwrap(),
            directory
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            filter(&requests[0]),
            json!({"filterActive": {"value": true}})
        );
        assert_eq!(
            requests[0].url.query_pairs().collect::<Vec<_>>(),
            vec![("view".into(), "players_wl".into())]
        );
    }

    #[tokio::test]
    async fn free_agent_fallback_reuses_history_for_ratings() {
        let server = MockServer::start().await;
        Mock::given(path(LEAGUE_PATH))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .and(query_param("view", "kona_player_info"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"players": []}])))
            .expect(1)
            .mount(&server)
            .await;
        schedules(&server).await;
        Mock::given(path(HISTORY_PATH))
            .and(query_param("view", "mPositionalRatings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{}])))
            .expect(1)
            .mount(&server)
            .await;
        let request = FreeAgentRequest::resolve(&snapshot(), &FreeAgentOptions::default()).unwrap();
        request
            .fetch(&mut transport(&server, Season(2024)))
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.url.path())
                .collect::<Vec<_>>(),
            vec![LEAGUE_PATH, HISTORY_PATH, SEASON_PATH, HISTORY_PATH]
        );
        assert_eq!(filter(&requests[0]), filter(&requests[1]));
        assert!(
            requests[3]
                .url
                .query_pairs()
                .any(|(key, value)| key == "seasonId" && value == "2024")
        );
    }

    #[tokio::test]
    async fn auxiliary_errors_stop_free_agents_and_cards() {
        let server = MockServer::start().await;
        Mock::given(path(LEAGUE_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"players": []})))
            .mount(&server)
            .await;
        Mock::given(path(SEASON_PATH))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let request = FreeAgentRequest::resolve(&snapshot(), &FreeAgentOptions::default()).unwrap();
        assert!(matches!(
            request.fetch(&mut transport(&server, Season(2024))).await,
            Err(Error::Http { status: 500 })
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert!(matches!(
            fetch_player_cards(
                &mut transport(&server, Season(2024)),
                Season(2024),
                &[PlayerId(1)],
                ScoringPeriod(18)
            )
            .await,
            Err(Error::Http { status: 500 })
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 4);
    }

    #[tokio::test]
    async fn card_fallback_is_reused_by_later_batches_and_keeps_filters() {
        let server = MockServer::start().await;
        Mock::given(path(LEAGUE_PATH))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"players": []}])))
            .expect(2)
            .mount(&server)
            .await;
        schedules(&server).await;
        let ids: Vec<_> = (1..=41).map(PlayerId).collect();
        fetch_player_cards(
            &mut transport(&server, Season(2024)),
            Season(2024),
            &ids,
            ScoringPeriod(18),
        )
        .await
        .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|request| request.url.path())
                .collect::<Vec<_>>(),
            vec![LEAGUE_PATH, HISTORY_PATH, HISTORY_PATH, SEASON_PATH]
        );
        assert_eq!(filter(&requests[0]), filter(&requests[1]));
        assert_eq!(
            filter(&requests[2])["players"]["filterIds"]["value"],
            json!([41])
        );
        assert!(
            !requests[3]
                .url
                .query_pairs()
                .any(|(key, _)| key == "seasonId")
        );
    }

    #[tokio::test]
    async fn directory_denial_never_falls_back_to_a_league() {
        let server = MockServer::start().await;
        Mock::given(path(format!("{SEASON_PATH}/players")))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        assert!(matches!(
            fetch_player_directory(&transport(&server, Season(2024))).await,
            Err(Error::Http { status: 401 })
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn requests_match_python_oracle_with_explicit_qb_zero_correction() {
        let oracle: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/players/python_requests.json"
        ))
        .unwrap();
        let mut snapshot = snapshot();
        snapshot.current_week = ScoringPeriod(7);
        for case in oracle["cases"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let is_free_agent = name.starts_with("free_agents_");
            let is_card = matches!(
                name,
                "player_card_id" | "player_card_many" | "player_card_empty"
            );
            if !is_free_agent && !is_card && name != "player_directory" {
                // Name lookup intentionally returns all duplicate-name IDs;
                // shared pool pagination belongs to a later history milestone.
                assert!(
                    matches!(
                        name,
                        "player_card_name"
                            | "player_card_unknown_name"
                            | "player_card_case_sensitive_name"
                            | "shared_player_pool_pagination"
                    ),
                    "unhandled Python request case {name}"
                );
                continue;
            }
            let server = MockServer::start().await;
            let rows: Vec<_> = case["result_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|id| json!({"player": {"id": id}}))
                .collect();
            Mock::given(path(LEAGUE_PATH))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"players": rows})))
                .mount(&server)
                .await;
            schedules(&server).await;
            Mock::given(path(format!("{SEASON_PATH}/players")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
                .mount(&server)
                .await;
            let mut transport = transport(&server, Season(2024));
            if name == "free_agents_historical_gate" {
                let mut historical = snapshot.clone();
                historical.season =
                    Season(u16::try_from(case["season"].as_u64().unwrap()).unwrap());
                assert!(matches!(
                    FreeAgentRequest::resolve(&historical, &FreeAgentOptions::default()),
                    Err(Error::Configuration(_))
                ));
            } else if is_free_agent {
                let args = &case["arguments"];
                let mut slots = Vec::new();
                if let Some(position) = args["position"].as_str() {
                    slots.push(match position {
                        "WR" => SlotId(4),
                        "QB" => SlotId(0),
                        _ => panic!("uncharacterized position {position}"),
                    });
                }
                if let Some(id) = args["position_id"].as_u64() {
                    slots.push(SlotId(u32::try_from(id).unwrap()));
                }
                let options = FreeAgentOptions {
                    week: args["week"]
                        .as_u64()
                        .map(|week| ScoringPeriod(u32::try_from(week).unwrap())),
                    limit: args["size"]
                        .as_u64()
                        .map(|size| u32::try_from(size).unwrap())
                        .unwrap_or(50),
                    offset: 0,
                    slots,
                };
                FreeAgentRequest::resolve(&snapshot, &options)
                    .unwrap()
                    .fetch(&mut transport)
                    .await
                    .unwrap();
            } else if is_card {
                let value = &case["arguments"]["playerId"];
                let ids: Vec<_> = if let Some(ids) = value.as_array() {
                    ids.iter()
                        .map(|id| PlayerId(id.as_i64().unwrap()))
                        .collect()
                } else {
                    vec![PlayerId(value.as_i64().unwrap())]
                };
                fetch_player_cards(&mut transport, Season(2024), &ids, ScoringPeriod(17))
                    .await
                    .unwrap();
            } else {
                fetch_player_directory(&transport).await.unwrap();
            }
            let requests = server.received_requests().await.unwrap();
            let trace: Vec<Value> = requests.iter().map(|request| {
                let mut query = serde_json::Map::new();
                for (key, value) in request.url.query_pairs() {
                    query.entry(key.into_owned()).or_insert_with(|| json!([])).as_array_mut().unwrap().push(json!(value));
                }
                let filter = request.headers.get("x-fantasy-filter").map(|header| serde_json::from_slice::<Value>(header.as_bytes()).unwrap()).unwrap_or(Value::Null);
                json!({"method": request.method.as_str(), "path": request.url.path(), "query": query, "fantasy_filter": filter})
            }).collect();
            let mut expected = case["requests"].clone();
            if name == "free_agents_explicit_qb_zero" {
                // Authorized correction: Python's truthiness check discards
                // numeric QB0. Every other trace field still matches exactly.
                assert_eq!(
                    expected[0]["fantasy_filter"]["players"]["filterSlotIds"]["value"],
                    json!([])
                );
                expected[0]["fantasy_filter"]["players"]["filterSlotIds"]["value"] = json!([0]);
            }
            assert_eq!(json!(trace), expected, "request case {name}");
        }
    }
}
