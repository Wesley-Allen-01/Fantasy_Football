use crate::football::scoreboard_from_value;
use crate::transport::EspnTransport;
use crate::{Error, LeagueId, LeagueSnapshot, Matchup, MatchupPeriod, Result, Season};
use std::fmt;
use std::time::Duration;

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
