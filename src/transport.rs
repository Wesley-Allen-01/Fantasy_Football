use crate::{Credentials, Error, LeagueId, Result, ScoringPeriod, Season};
use reqwest::{Client, StatusCode, Url};
use serde_json::Value;

#[derive(Clone, Copy)]
enum LeagueRoute {
    Season,
    History,
}

impl LeagueRoute {
    fn alternate(self) -> Self {
        match self {
            Self::Season => Self::History,
            Self::History => Self::Season,
        }
    }
}

/// Route discovery belongs to one league and season. A mutable handle prevents
/// overlapping fallback attempts from overwriting a successfully discovered route.
pub(crate) struct EspnTransport {
    http: Client,
    base_url: Url,
    league_id: LeagueId,
    season: Season,
    credentials: Option<Credentials>,
    route: LeagueRoute,
}

impl EspnTransport {
    pub(crate) fn new(
        http: Client,
        base_url: Url,
        league_id: LeagueId,
        season: Season,
        credentials: Option<Credentials>,
    ) -> Result<Self> {
        if !matches!(base_url.scheme(), "http" | "https")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || !base_url.path().ends_with("/apis/v3/games/")
        {
            return Err(Error::Configuration(
                "base URL must be HTTP(S), end with /apis/v3/games/, and have no credentials, query, or fragment".into(),
            ));
        }
        Ok(Self {
            http,
            base_url,
            league_id,
            season,
            credentials,
            route: if season.0 < 2018 {
                LeagueRoute::History
            } else {
                LeagueRoute::Season
            },
        })
    }

    pub(crate) async fn league_get(
        &mut self,
        views: &[&str],
        scoring_period: Option<ScoringPeriod>,
        filter: Option<&Value>,
        extension: &str,
    ) -> Result<Value> {
        validate_extension(extension)?;
        let response = self
            .send(self.route, views, scoring_period, filter, extension)
            .await?;
        match response.status() {
            StatusCode::OK => decode(response).await,
            StatusCode::UNAUTHORIZED => {
                let alternate = self.route.alternate();
                let response = self
                    .send(alternate, views, scoring_period, filter, extension)
                    .await?;
                if response.status() != StatusCode::OK {
                    return Err(Error::AccessDenied {
                        missing_credentials: self.credentials.is_none(),
                    });
                }
                // Read, decode, and normalize first. Network, JSON, empty-list,
                // and cancellation failures must retain the original route.
                let value = decode(response).await?;
                self.route = alternate;
                Ok(value)
            }
            StatusCode::NOT_FOUND if is_communication(extension) => {
                Ok(serde_json::json!({"topics": []}))
            }
            StatusCode::NOT_FOUND => Err(Error::InvalidLeague {
                league_id: self.league_id,
            }),
            status => Err(Error::Http {
                status: status.as_u16(),
            }),
        }
    }

    async fn send(
        &self,
        route: LeagueRoute,
        views: &[&str],
        scoring_period: Option<ScoringPeriod>,
        filter: Option<&Value>,
        extension: &str,
    ) -> Result<reqwest::Response> {
        let mut url = self.base_url.clone();
        {
            // The constructor validated a hierarchical HTTP(S) URL.
            let mut path = url.path_segments_mut().map_err(|_| {
                Error::Configuration("base URL does not support path segments".into())
            })?;
            path.pop_if_empty().push("ffl");
            match route {
                LeagueRoute::Season => {
                    path.extend([
                        "seasons",
                        &self.season.to_string(),
                        "segments",
                        "0",
                        "leagues",
                        &self.league_id.to_string(),
                    ]);
                }
                LeagueRoute::History => {
                    path.extend(["leagueHistory", &self.league_id.to_string()]);
                }
            }
            if !extension.is_empty() {
                path.extend(extension[1..].split('/'));
            }
        }
        {
            let mut query = url.query_pairs_mut();
            if matches!(route, LeagueRoute::History) {
                query.append_pair("seasonId", &self.season.to_string());
            }
            for view in views {
                query.append_pair("view", view);
            }
            if let Some(period) = scoring_period {
                query.append_pair("scoringPeriodId", &period.to_string());
            }
        }
        let mut request = self.http.get(url);
        if let Some(credentials) = &self.credentials {
            request = request.header(reqwest::header::COOKIE, credentials.cookie_header());
        }
        if let Some(filter) = filter {
            request = request.header("x-fantasy-filter", filter.to_string());
        }
        Ok(request.send().await?)
    }
}

fn validate_extension(extension: &str) -> Result<()> {
    if extension.is_empty() {
        return Ok(());
    }
    let suffix = extension.strip_suffix('/').unwrap_or(extension);
    let valid = suffix.starts_with('/')
        && suffix[1..].split('/').all(|segment| {
            !matches!(segment, "." | "..")
                && segment.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
                })
                && !segment.is_empty()
        });
    if valid {
        Ok(())
    } else {
        Err(Error::Configuration(
            "extension must be a slash-prefixed path suffix without internal empty segments, traversal, encoded separators, query, or fragment".into(),
        ))
    }
}

fn is_communication(extension: &str) -> bool {
    extension.split('/').any(|part| part == "communication")
}

async fn decode(response: reqwest::Response) -> Result<Value> {
    let bytes = response.bytes().await?;
    let value: Value = serde_json::from_slice(&bytes)?;
    match value {
        Value::Array(values) => values
            .into_iter()
            .next()
            .ok_or_else(|| Error::InvalidResponse {
                context: "league response contained an empty history array".into(),
            }),
        value => Ok(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    const SEASON_PATH: &str = "/apis/v3/games/ffl/seasons/2024/segments/0/leagues/123";
    const HISTORY_PATH: &str = "/apis/v3/games/ffl/leagueHistory/123";

    fn transport(
        server: &MockServer,
        season: u16,
        credentials: Option<Credentials>,
    ) -> EspnTransport {
        EspnTransport::new(
            Client::builder()
                .no_proxy()
                .timeout(Duration::from_millis(300))
                .build()
                .unwrap(),
            Url::parse(&format!("{}/apis/v3/games/", server.uri())).unwrap(),
            LeagueId(123),
            Season(season),
            credentials,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn fallback_preserves_filter_period_cookie_and_extension() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("{SEASON_PATH}/communication")))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("{HISTORY_PATH}/communication")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(vec![serde_json::json!({"topics": []})]),
            )
            .expect(2)
            .mount(&server)
            .await;
        let credentials = Credentials::new("s2-test", "{swid-test}").unwrap();
        let mut transport = transport(&server, 2024, Some(credentials));
        let filter =
            serde_json::json!({"topics": {"limit": 25, "sortMessageDate": {"sortAsc": false}}});
        for _ in 0..2 {
            assert_eq!(
                transport
                    .league_get(
                        &["a", "b"],
                        Some(ScoringPeriod(8)),
                        Some(&filter),
                        "/communication"
                    )
                    .await
                    .unwrap(),
                serde_json::json!({"topics": []})
            );
        }
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 3);
        for (index, request) in requests.iter().enumerate() {
            let pairs: Vec<_> = request
                .url
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect();
            assert_eq!(
                pairs
                    .iter()
                    .filter(|(key, _)| key == "view")
                    .map(|(_, value)| value.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "b"]
            );
            assert!(pairs.contains(&("scoringPeriodId".into(), "8".into())));
            assert_eq!(
                pairs
                    .iter()
                    .find(|(key, _)| key == "seasonId")
                    .map(|(_, value)| value.as_str()),
                if index == 0 { None } else { Some("2024") }
            );
            assert_eq!(
                serde_json::from_slice::<Value>(request.headers["x-fantasy-filter"].as_bytes())
                    .unwrap(),
                filter
            );
            let cookie = request.headers["cookie"].to_str().unwrap();
            assert!(cookie.contains("espn_s2=s2-test"));
            assert!(cookie.contains("SWID={swid-test}"));
        }
    }

    #[tokio::test]
    async fn failed_fallbacks_keep_primary_route() {
        for (status, body, expected) in [
            (403, "", "access"),
            (200, "<html>", "decode"),
            (200, "[]", "empty"),
        ] {
            let server = MockServer::start().await;
            let primary_requests = Arc::new(AtomicUsize::new(0));
            let counter = primary_requests.clone();
            Mock::given(path(SEASON_PATH))
                .respond_with(move |_: &wiremock::Request| {
                    if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                        ResponseTemplate::new(401)
                    } else {
                        ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 123}))
                    }
                })
                .mount(&server)
                .await;
            Mock::given(path(HISTORY_PATH))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .expect(1)
                .mount(&server)
                .await;
            let mut transport = transport(&server, 2024, None);
            let error = transport.league_get(&[], None, None, "").await.unwrap_err();
            match expected {
                "access" => assert!(matches!(
                    error,
                    Error::AccessDenied {
                        missing_credentials: true
                    }
                )),
                "decode" => assert!(matches!(error, Error::Decode(_))),
                "empty" => assert!(matches!(error, Error::InvalidResponse { .. })),
                _ => unreachable!(),
            }
            assert_eq!(
                transport.league_get(&[], None, None, "").await.unwrap(),
                serde_json::json!({"id": 123})
            );
            assert_eq!(primary_requests.load(Ordering::SeqCst), 2);
        }
    }

    #[tokio::test]
    async fn timeout_during_fallback_keeps_primary_route() {
        let server = MockServer::start().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let primary = counter.clone();
        Mock::given(path(SEASON_PATH))
            .respond_with(move |_: &wiremock::Request| {
                if primary.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(401)
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 123}))
                }
            })
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 123}))
                    .set_delay(Duration::from_secs(1)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut transport = transport(&server, 2024, None);
        assert!(
            matches!(transport.league_get(&[], None, None, "").await, Err(Error::Network(error)) if error.is_timeout())
        );
        assert!(transport.league_get(&[], None, None, "").await.is_ok());
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn connection_failure_during_fallback_keeps_primary_route() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            // Close the listener before replying so the alternate route's new
            // connection is deterministically refused.
            drop(listener);
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 4096];
            let bytes_read = socket.read(&mut buffer).unwrap();
            assert!(
                bytes_read > 0,
                "fallback must reach the test server before it disconnects"
            );
            socket
                .write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        let mut transport = EspnTransport::new(
            Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            Url::parse(&format!("http://{address}/apis/v3/games/")).unwrap(),
            LeagueId(123),
            Season(2024),
            None,
        )
        .unwrap();
        assert!(
            matches!(transport.league_get(&[], None, None, "").await, Err(Error::Network(error)) if error.is_connect())
        );
        assert!(matches!(transport.route, LeagueRoute::Season));
        server.join().unwrap();
    }

    #[tokio::test]
    async fn cancellation_during_fallback_keeps_primary_route() {
        let server = MockServer::start().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let primary = counter.clone();
        Mock::given(path(SEASON_PATH))
            .respond_with(move |_: &wiremock::Request| {
                if primary.fetch_add(1, Ordering::SeqCst) == 0 {
                    ResponseTemplate::new(401)
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": 123}))
                }
            })
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": 123}))
                    .set_delay(Duration::from_secs(1)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut transport = transport(&server, 2024, None);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(150),
                transport.league_get(&[], None, None, "")
            )
            .await
            .is_err()
        );
        assert!(matches!(transport.route, LeagueRoute::Season));
        assert!(transport.league_get(&[], None, None, "").await.is_ok());
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn primary_status_errors_and_communication_404_are_distinct() {
        let server = MockServer::start().await;
        Mock::given(path(SEASON_PATH))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path(format!("{SEASON_PATH}/communication")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path(format!("{SEASON_PATH}/communication/")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path(format!("{SEASON_PATH}/other")))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        Mock::given(path(format!("{SEASON_PATH}/not-communication")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let mut transport = transport(&server, 2024, None);
        assert!(matches!(
            transport.league_get(&[], None, None, "").await,
            Err(Error::InvalidLeague {
                league_id: LeagueId(123)
            })
        ));
        assert_eq!(
            transport
                .league_get(&[], None, None, "/communication")
                .await
                .unwrap(),
            serde_json::json!({"topics": []})
        );
        assert_eq!(
            transport
                .league_get(&[], None, None, "/communication/")
                .await
                .unwrap(),
            serde_json::json!({"topics": []})
        );
        assert!(matches!(
            transport.league_get(&[], None, None, "/other").await,
            Err(Error::Http { status: 500 })
        ));
        assert!(matches!(
            transport
                .league_get(&[], None, None, "/not-communication")
                .await,
            Err(Error::InvalidLeague { .. })
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 5);
    }

    #[tokio::test]
    async fn denied_pair_is_reported_after_one_fallback_even_on_404() {
        let server = MockServer::start().await;
        Mock::given(path(SEASON_PATH))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path(HISTORY_PATH))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let mut transport = transport(&server, 2024, Some(Credentials::new("s2", "swid").unwrap()));
        assert!(matches!(
            transport.league_get(&[], None, None, "").await,
            Err(Error::AccessDenied {
                missing_credentials: false
            })
        ));
    }

    #[tokio::test]
    async fn invalid_extensions_fail_before_requests() {
        let server = MockServer::start().await;
        let mut transport = transport(&server, 2024, None);
        for extension in [
            "communication",
            "/../players",
            "/./players",
            "//host",
            "/a//b",
            "/a?secret=b",
            "/a#b",
            "/%2fplayers",
            "/%2e%2e/players",
            "/a\\b",
            "https://example.com",
        ] {
            assert!(
                matches!(
                    transport.league_get(&[], None, None, extension).await,
                    Err(Error::Configuration(_))
                ),
                "accepted {extension}"
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn invalid_base_urls_are_rejected() {
        for url in [
            "file:///apis/v3/games/",
            "https://example.com/wrong/",
            "https://user:password@example.com/apis/v3/games/",
            "https://example.com/apis/v3/games/?a=b",
            "https://example.com/apis/v3/games/#fragment",
        ] {
            assert!(matches!(
                EspnTransport::new(
                    Client::new(),
                    Url::parse(url).unwrap(),
                    LeagueId(123),
                    Season(2024),
                    None
                ),
                Err(Error::Configuration(_))
            ));
        }
    }
}
