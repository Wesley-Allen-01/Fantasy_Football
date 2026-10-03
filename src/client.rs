use crate::football::scoreboard_from_value;
use crate::player_request::{FreeAgentRequest, fetch_player_cards, fetch_player_directory};
use crate::transport::EspnTransport;
use crate::weekly_request::WeeklyRequest;
use crate::{
    BoxScoreContext, Error, FreeAgentContext, FreeAgentOptions, FreeAgentPage, LeagueId,
    LeagueSnapshot, Matchup, MatchupPeriod, PlayerCard, PlayerDirectory, PlayerId,
    PlayerTeamHistory, Result, ScoringPeriod, Season, WeeklyBoxScores,
};
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const BASE_URL: &str = "https://lm-api-reads.fantasy.espn.com/apis/v3/games/";
const LEAGUE_VIEWS: &[&str] = &["mTeam", "mRoster", "mMatchup", "mSettings", "mStandings"];

/// The paired cookies needed to access a private ESPN league.
///
/// Credentials are never included in Debug output. They are supplied by the
/// caller; this library does not perform account login.
#[derive(Clone)]
pub struct Credentials {
    espn_s2: String,
    swid: String,
}

impl Credentials {
    pub fn new(espn_s2: impl Into<String>, swid: impl Into<String>) -> Result<Self> {
        let credentials = Self {
            espn_s2: espn_s2.into(),
            swid: swid.into(),
        };
        for value in [&credentials.espn_s2, &credentials.swid] {
            if value.is_empty()
                || value
                    .bytes()
                    .any(|byte| byte <= 0x20 || byte >= 0x7f || byte == b';')
            {
                return Err(Error::Configuration(
                    "both espn_s2 and SWID must be nonempty cookie values without whitespace or separators".into(),
                ));
            }
        }
        Ok(credentials)
    }

    pub(crate) fn cookie_header(&self) -> String {
        format!("espn_s2={}; SWID={}", self.espn_s2, self.swid)
    }
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("espn_s2", &"[redacted]")
            .field("swid", &"[redacted]")
            .finish()
    }
}

/// A reusable HTTP client. Constructing it does not contact ESPN.
#[derive(Clone, Debug)]
pub struct Client {
    http: reqwest::Client,
    base_url: reqwest::Url,
    credentials: Option<Credentials>,
}

impl Client {
    pub fn builder() -> ClientBuilder {
        ClientBuilder::default()
    }

    /// Create a league handle with its own historical-route selection state.
    /// This performs no network requests.
    pub fn league(&self, league_id: LeagueId, season: Season) -> Result<LeagueHandle> {
        if league_id.0 == 0 || season.0 == 0 {
            return Err(Error::Configuration(
                "league ID and season must be nonzero".into(),
            ));
        }
        let transport = EspnTransport::new(
            self.http.clone(),
            self.base_url.clone(),
            league_id,
            season,
            self.credentials.clone(),
        )?;
        Ok(LeagueHandle {
            transport,
            league_id,
            season,
            snapshot: None,
        })
    }
}

#[derive(Debug)]
pub struct ClientBuilder {
    base_url: String,
    credentials: Option<Credentials>,
    timeout: Duration,
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self {
            base_url: BASE_URL.into(),
            credentials: None,
            timeout: Duration::from_secs(30),
        }
    }
}

impl ClientBuilder {
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Override the read-service base URL for a mock server or compatible proxy.
    /// The URL must end with `/apis/v3/games/` and contain no query or user info.
    pub fn base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Set a finite timeout for each HTTP request, including fallback requests.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn build(self) -> Result<Client> {
        if self.timeout.is_zero() {
            return Err(Error::Configuration(
                "timeout must be greater than zero".into(),
            ));
        }
        let base_url = reqwest::Url::parse(&self.base_url)
            .map_err(|_| Error::Configuration("invalid read-service URL".into()))?;
        if !matches!(base_url.scheme(), "https" | "http")
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || !base_url.path().ends_with("/apis/v3/games/")
        {
            return Err(Error::Configuration(
                "read-service URL must use HTTP(S), end in /apis/v3/games/, and omit user info, query and fragment".into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(self.timeout)
            // A redirected request must not carry private league cookies to a
            // different host. Treat redirects as explicit HTTP errors.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Client {
            http,
            base_url,
            credentials: self.credentials,
        })
    }
}

/// A football league reader with isolated fallback state and a loaded snapshot.
///
/// Methods take `&mut self` to serialize route discovery and snapshot updates.
pub struct LeagueHandle {
    transport: EspnTransport,
    league_id: LeagueId,
    season: Season,
    snapshot: Option<LeagueSnapshot>,
}

impl LeagueHandle {
    pub fn league_id(&self) -> LeagueId {
        self.league_id
    }

    pub fn season(&self) -> Season {
        self.season
    }

    /// The last successfully loaded snapshot; failed refreshes leave it intact.
    pub fn snapshot(&self) -> Option<&LeagueSnapshot> {
        self.snapshot.as_ref()
    }

    /// Load settings, teams, rosters and schedules and return an owned snapshot.
    ///
    /// Unlike Python's eager constructor, this basic load does not request the
    /// full professional player directory, professional schedule or draft.
    pub async fn fetch(&mut self) -> Result<LeagueSnapshot> {
        let snapshot = self.load_snapshot().await?;
        self.snapshot = Some(snapshot.clone());
        Ok(snapshot)
    }

    /// Replace the snapshot only when the entire response parses successfully.
    pub async fn refresh(&mut self) -> Result<&LeagueSnapshot> {
        let snapshot = self.load_snapshot().await?;
        Ok(self.snapshot.insert(snapshot))
    }

    async fn load_snapshot(&mut self) -> Result<LeagueSnapshot> {
        let response = self
            .transport
            .league_get(LEAGUE_VIEWS, None, None, "")
            .await?;
        LeagueSnapshot::from_value(&response, self.league_id, self.season)
    }

    /// Read matchup totals for one matchup period, preserving schedule order.
    ///
    /// With no explicit period, a loaded snapshot is required. Python defaults
    /// its football scoreboard to `current_week`; the same default is retained
    /// here, even when a league has multi-week matchup periods.
    pub async fn scoreboard(&mut self, period: Option<MatchupPeriod>) -> Result<Vec<Matchup>> {
        if period.is_some_and(|period| period.0 == 0) {
            return Err(Error::Configuration(
                "explicit matchup period must be greater than zero".into(),
            ));
        }
        let period = period
            .or_else(|| {
                self.snapshot
                    .as_ref()
                    .map(|snapshot| MatchupPeriod(snapshot.current_week.0))
            })
            .ok_or_else(|| {
                Error::Configuration("load the league or supply an explicit matchup period".into())
            })?;
        let response = self
            .transport
            .league_get(&["mMatchupScore"], None, None, "")
            .await?;
        scoreboard_from_value(&response, period)
    }

    /// Fetch weekly lineups, actual/projected points and NFL game context.
    ///
    /// Load a snapshot first. Seasons before 2019 and an explicit zero week
    /// return configuration errors. Future weeks use the loaded current week,
    /// matching Python. This does not replace the loaded roster or snapshot.
    ///
    /// ```no_run
    /// use espn_fantasy_football::{Client, LeagueId, ScoringPeriod, Season, TeamId};
    /// # async fn example() -> espn_fantasy_football::Result<()> {
    /// let mut league = Client::builder().build()?.league(LeagueId(394172912), Season(2026))?;
    /// league.fetch().await?;
    /// let weekly = league.box_scores(Some(ScoringPeriod(4))).await?;
    /// if let Some(matchup) = weekly.for_team(TeamId(1)) {
    ///     println!("{matchup:#?}");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub async fn box_scores(&mut self, week: Option<ScoringPeriod>) -> Result<WeeklyBoxScores> {
        let mut history = PlayerTeamHistory::new(self.season);
        self.box_scores_with_history(week, &mut history).await
    }

    /// Fetch weekly box scores using explicit historical NFL team evidence.
    ///
    /// Reuse a history while reading weeks chronologically to resolve traded
    /// players with no actual statistics during a bye. Only actual team
    /// evidence updates it. A failed request or parse leaves history unchanged.
    /// Without this method, each call starts with empty history.
    pub async fn box_scores_with_history(
        &mut self,
        week: Option<ScoringPeriod>,
        history: &mut PlayerTeamHistory,
    ) -> Result<WeeklyBoxScores> {
        if self.season.0 < 2019 {
            return Err(Error::Configuration(
                "football box scores require a season of 2019 or later".into(),
            ));
        }
        if history.season() != self.season {
            return Err(Error::Configuration(
                "player team history belongs to a different season".into(),
            ));
        }
        let snapshot = self.snapshot.as_ref().ok_or_else(|| {
            Error::Configuration("load the league before requesting weekly box scores".into())
        })?;
        let request = WeeklyRequest::resolve(snapshot, week)?;
        let payloads = request.fetch(&mut self.transport).await?;
        let now_unix_ms = unix_now_ms()?;
        WeeklyBoxScores::from_values(
            &payloads.box_scores,
            &payloads.pro_schedule,
            &payloads.positional_ratings,
            BoxScoreContext {
                season: self.season,
                scoring_period: request.scoring_period,
                matchup_period: request.matchup_period,
                now_unix_ms,
                history,
            },
        )
    }
}

impl LeagueHandle {
    /// Read one page of free agents and waiver players with weekly statistics.
    /// Load a snapshot first. Defaults match Python: current week, 50 players,
    /// all slots, descending ownership. Explicit future weeks are preserved.
    /// Pagination is caller-controlled; this method never loops over pages.
    /// No query replaces the loaded roster or snapshot.
    ///
    /// ```no_run
    /// use espn_fantasy_football::{Client, FreeAgentOptions, LeagueId, Season, SlotId};
    /// # async fn example() -> espn_fantasy_football::Result<()> {
    /// let mut league = Client::builder().build()?.league(LeagueId(394172912), Season(2026))?;
    /// league.fetch().await?;
    /// let quarterbacks = league.free_agents(FreeAgentOptions {
    ///     slots: vec![SlotId(0)], ..Default::default()
    /// }).await?;
    /// println!("{} available quarterbacks", quarterbacks.players.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn free_agents(&mut self, options: FreeAgentOptions) -> Result<FreeAgentPage> {
        let snapshot = self.snapshot.as_ref().ok_or_else(|| {
            Error::Configuration("load the league before requesting free agents".into())
        })?;
        let request = FreeAgentRequest::resolve(snapshot, &options)?;
        let payloads = request.fetch(&mut self.transport).await?;
        FreeAgentPage::from_values(
            &payloads.players,
            &payloads.pro_schedule,
            &payloads.positional_ratings,
            FreeAgentContext {
                season: self.season,
                scoring_period: request.scoring_period,
                now_unix_ms: unix_now_ms()?,
                offset: options.offset,
                limit: options.limit,
            },
        )
    }

    /// Fetch one player by ESPN ID. A valid empty response returns None.
    pub async fn player_by_id(&mut self, id: PlayerId) -> Result<Option<PlayerCard>> {
        Ok(self.players_by_ids(&[id]).await?.into_iter().next())
    }

    /// Fetch player cards in batches of at most 40 IDs, preserving server
    /// response order within each batch. Duplicate requested IDs are removed;
    /// missing players are omitted. Empty input performs no network requests.
    /// A nonempty query needs a loaded snapshot for the final stat period.
    pub async fn players_by_ids(&mut self, ids: &[PlayerId]) -> Result<Vec<PlayerCard>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        if ids.iter().any(|id| id.0 == 0) {
            return Err(Error::Configuration("player IDs must be nonzero".into()));
        }
        let final_period = self
            .snapshot
            .as_ref()
            .ok_or_else(|| {
                Error::Configuration("load the league before requesting player cards".into())
            })?
            .final_scoring_period;
        let payloads =
            fetch_player_cards(&mut self.transport, self.season, ids, final_period).await?;
        let cards =
            PlayerCard::from_values(&payloads.players, &payloads.pro_schedule, self.season)?;
        if cards.iter().any(|card| !ids.contains(&card.player.id)) {
            return Err(Error::InvalidResponse {
                context: "player-card response contains an ID that was not requested".into(),
            });
        }
        Ok(cards)
    }

    /// Explicitly fetch the active season player directory; no league load is
    /// needed. The returned directory can be reused for local exact-name queries.
    pub async fn player_directory(&self) -> Result<PlayerDirectory> {
        let value = fetch_player_directory(&self.transport).await?;
        PlayerDirectory::from_value(&value, self.season)
    }

    /// Resolve an exact, case-sensitive name to every matching active player ID
    /// and fetch its cards. This explicitly reloads the directory each time.
    /// An unknown name returns an empty list without card or schedule requests.
    ///
    /// ```no_run
    /// use espn_fantasy_football::{Client, LeagueId, PlayerId, Season};
    /// # async fn example() -> espn_fantasy_football::Result<()> {
    /// let mut league = Client::builder().build()?.league(LeagueId(394172912), Season(2026))?;
    /// league.fetch().await?;
    /// let card = league.player_by_id(PlayerId(3117251)).await?;
    /// let matching_cards = league.players_named("Exact Player Name").await?;
    /// println!("{card:#?} {} name matches", matching_cards.len());
    /// # Ok(())
    /// # }
    /// ```
    pub async fn players_named(&mut self, name: &str) -> Result<Vec<PlayerCard>> {
        if name.trim().is_empty() {
            return Err(Error::Configuration("player name must not be blank".into()));
        }
        if self.snapshot.is_none() {
            return Err(Error::Configuration(
                "load the league before requesting player cards".into(),
            ));
        }
        let directory = self.player_directory().await?;
        self.players_by_ids(&directory.ids_named(name)).await
    }
}

fn unix_now_ms() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Configuration("system clock precedes the Unix epoch".into()))?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Configuration("system timestamp is out of range".into()))
}

impl fmt::Debug for LeagueHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeagueHandle")
            .field("league_id", &self.league_id)
            .field("season", &self.season)
            .field("loaded", &self.snapshot.is_some())
            .finish_non_exhaustive()
    }
}
