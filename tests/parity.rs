//! Cross-language semantic checks. Expected values come from the unchanged Python
//! implementation under closed HTTP mocks, not Rust's conversion routines.
use espn_fantasy_football::{
    LeagueId, LeagueSnapshot, Matchup, MatchupPeriod, Player, PlayerId, Season,
    football::{pro_team_abbreviation, scoreboard_from_value, slot_label},
};
use serde_json::{Map, Value, json};
use std::{fs, path::Path};

fn player_projection(player: &Player) -> Value {
    let mut stats = Map::new();
    for (period, lines) in &player.stats {
        let mut fields = Map::new();
        if let Some(actual) = &lines.actual {
            fields.insert("points".into(), json!(actual.points));
            fields.insert("avg_points".into(), json!(actual.average_points));
        }
        if let Some(projected) = &lines.projected {
            fields.insert("projected_points".into(), json!(projected.points));
            fields.insert(
                "projected_avg_points".into(),
                json!(projected.average_points),
            );
        }
        stats.insert(period.to_string(), Value::Object(fields));
    }
    json!({
        "id": player.id,
        "name": player.name,
        "position": player.position.and_then(slot_label),
        "pro_team": pro_team_abbreviation(player.pro_team),
        "eligible_slots": player.eligible_slots.iter().map(|id| slot_label(*id)).collect::<Vec<_>>(),
        "total_points": player.total_points,
        "projected_total_points": player.projected_total_points,
        "avg_points": player.avg_points,
        "projected_avg_points": player.projected_avg_points,
        "stats": stats,
    })
}

fn matchup_projection(matchup: &Matchup) -> Value {
    json!({
        "period": matchup.period,
        "home_team": matchup.home_team,
        "away_team": matchup.away_team,
        "home_score": matchup.home_score,
        "away_score": matchup.away_score,
        "winner": matchup.winner,
        "matchup_type": matchup.matchup_type,
        "is_playoff": matchup.is_playoff,
    })
}

fn snapshot_projection(snapshot: &LeagueSnapshot, expected: &Value) -> Value {
    let teams: Vec<_> = snapshot
        .teams
        .iter()
        .map(|team| {
            // Full reference payloads contain larger rosters. Select precisely the
            // IDs in the Python golden, keeping source roster order in either case.
            let selected: Vec<_> = expected["teams"]
                .as_array()
                .unwrap()
                .iter()
                .find(|t| t["id"].as_u64() == Some(u64::from(team.id.0)))
                .unwrap()["roster"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| PlayerId(p["id"].as_i64().unwrap()))
                .collect();
            let roster: Vec<_> = team
                .roster
                .iter()
                .filter(|p| selected.contains(&p.id))
                .map(player_projection)
                .collect();
            let schedule: Vec<_> = team
                .schedule
                .iter()
                .map(|m| {
                    json!({
                        "period": m.period, "opponent": m.opponent, "score": m.score,
                        "outcome": m.outcome, "margin": m.margin,
                    })
                })
                .collect();
            json!({
                "id": team.id, "name": team.name, "abbreviation": team.abbreviation,
                "wins": team.wins, "losses": team.losses, "ties": team.ties,
                "points_for": team.points_for, "points_against": team.points_against,
                "standing": team.standing, "final_standing": team.final_standing,
                "roster": roster, "schedule": schedule,
            })
        })
        .collect();
    json!({
        "league_id": snapshot.league_id, "season": snapshot.season,
        "current_week": snapshot.current_week,
        "nfl_week": snapshot.nfl_week,
        "current_matchup_period": snapshot.current_matchup_period,
        "scoring_period": snapshot.scoring_period,
        "first_scoring_period": snapshot.first_scoring_period,
        "final_scoring_period": snapshot.final_scoring_period,
        "previous_seasons": snapshot.previous_seasons,
        "settings": {
            "name": snapshot.settings.name,
            "team_count": snapshot.settings.team_count,
            "scoring_type": snapshot.settings.scoring_type,
            "regular_season_matchup_count": snapshot.settings.reg_season_count,
            "playoff_team_count": snapshot.settings.playoff_team_count,
            "playoff_seed_tie_rule": snapshot.settings.playoff_seed_tie_rule,
            "median_scoring": snapshot.settings.median_scoring,
        },
        "teams": teams,
        "standings": snapshot.standings().iter().map(|team| team.id).collect::<Vec<_>>(),
    })
}

/// Compare numbers as exact binary values so Python integer zero and Rust float
/// zero are equivalent. No epsilon/tolerance masks rounding or scoring errors.
fn assert_semantically_equal(actual: &Value, expected: &Value, path: &str) {
    match (actual, expected) {
        (Value::Number(a), Value::Number(e)) => assert_eq!(a.as_f64(), e.as_f64(), "{path}"),
        (Value::Array(a), Value::Array(e)) => {
            assert_eq!(a.len(), e.len(), "{path}: array length");
            for (index, (a, e)) in a.iter().zip(e).enumerate() {
                assert_semantically_equal(a, e, &format!("{path}[{index}]"));
            }
        }
        (Value::Object(a), Value::Object(e)) => {
            assert_eq!(a.len(), e.len(), "{path}: field count");
            for (key, value) in e {
                assert_semantically_equal(
                    a.get(key).unwrap_or_else(|| panic!("{path}.{key} missing")),
                    value,
                    &format!("{path}.{key}"),
                );
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

fn check_case(season: u16, input: &Value, scoreboard: &Value, expected: &Value) {
    let snapshot = LeagueSnapshot::from_value(
        input,
        LeagueId(expected["league_id"].as_u64().unwrap()),
        Season(season),
    )
    .unwrap();
    let actual = snapshot_projection(&snapshot, expected);
    let mut league_expected = expected.clone();
    league_expected
        .as_object_mut()
        .unwrap()
        .remove("scoreboards");
    assert_semantically_equal(&actual, &league_expected, &format!("{season}.league"));
    for (period, expected_matchups) in expected["scoreboards"].as_object().unwrap() {
        let period = MatchupPeriod(period.parse().unwrap());
        let matchups = scoreboard_from_value(scoreboard, period).unwrap();
        let actual = json!(matchups.iter().map(matchup_projection).collect::<Vec<_>>());
        assert_semantically_equal(
            &actual,
            expected_matchups,
            &format!("{season}.scoreboard.{period}"),
        );
    }
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn historical_and_modern_football_match_python_goldens() {
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for season in [2015, 2018] {
        let input = read_json(fixture_dir.join(format!("football_{season}_league.json")));
        let scoreboard = read_json(fixture_dir.join(format!("football_{season}_scoreboard.json")));
        let expected = read_json(fixture_dir.join(format!("football_{season}_expected.json")));
        check_case(season, &input, &scoreboard, &expected);
    }
}

#[test]
fn full_reference_payloads_match_same_projection_when_available() {
    let Some(reference) = std::env::var_os("ESPN_PYTHON_REFERENCE") else {
        // Portable CI uses only compact fixtures. Set the variable to enable
        // validation against the original multi-megabyte reference inputs.
        return;
    };
    let reference = Path::new(&reference).join("tests/football/unit/data");
    let fixture_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    for season in [2015, 2018] {
        check_case(
            season,
            &read_json(reference.join(format!("league_{season}_data.json"))),
            &read_json(reference.join(format!("league_matchupScore_{season}.json"))),
            &read_json(fixture_dir.join(format!("football_{season}_expected.json"))),
        );
    }
}
