//! Python BoxScore/BoxPlayer oracle parity with a frozen UTC clock.
use espn_fantasy_football::{
    MatchupPeriod, PlayerId, ProTeamId, ScoringPeriod, Season, StatId,
    football::{
        BoxPlayer, BoxScoreContext, BoxTeam, PlayerTeamHistory, WeeklyBoxScores,
        pro_team_abbreviation, stat_label,
    },
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/weekly")
}

fn breakdown_projection(data: &BTreeMap<StatId, f64>, aliases: &[String]) -> Value {
    let mut fields = Map::new();
    for (id, number) in data {
        let label = stat_label(*id)
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_string());
        // Python aliases overwrite distinct raw IDs; that deliberate Rust
        // correction is outside label parity. Explicit provenance lists them.
        if !aliases.contains(&label) {
            fields.insert(label, json!(number));
        }
    }
    Value::Object(fields)
}

fn player_projection(player: &BoxPlayer, aliases: &[String]) -> Value {
    json!({
        "id": player.player.id,
        "name": player.player.name,
        "slot_position": player.slot_position_label(),
        "pro_team": pro_team_abbreviation(player.pro_team),
        "pro_opponent": player.pro_opponent.and_then(pro_team_abbreviation),
        "pro_pos_rank": player.pro_pos_rank,
        "game_date_unix_ms": player.game_date_unix_ms,
        "game_played": player.game_played,
        "on_bye_week": player.on_bye_week,
        "points": player.points,
        "projected_points": player.projected_points,
        "breakdown": breakdown_projection(&player.breakdown, aliases),
        "points_breakdown": breakdown_projection(&player.points_breakdown, aliases),
        "projected_breakdown": breakdown_projection(&player.projected_breakdown, aliases),
        "projected_points_breakdown": breakdown_projection(&player.projected_points_breakdown, aliases),
    })
}

fn side_projection(side: &BoxTeam, aliases: &[String]) -> Value {
    json!({
        "team_id": side.team_id,
        "score": side.score,
        "projected": side.projected,
        "lineup": side.lineup.iter().map(|player| player_projection(player, aliases)).collect::<Vec<_>>(),
    })
}

fn output_projection(
    scores: &WeeklyBoxScores,
    history: &PlayerTeamHistory,
    aliases: &[String],
) -> Value {
    json!({
        "season": scores.season,
        "scoring_period": scores.scoring_period,
        "matchup_period": scores.matchup_period,
        "matchups": scores.matchups.iter().map(|matchup| json!({
            "id": matchup.id,
            "home": matchup.home.as_ref().map(|side| side_projection(side, aliases)),
            "away": matchup.away.as_ref().map(|side| side_projection(side, aliases)),
            "matchup_type": matchup.matchup_type,
            "is_playoff": matchup.is_playoff,
        })).collect::<Vec<_>>(),
        "history": history.teams(),
    })
}

/// Exact binary numeric equality permits integer versus float representations
/// of the same value, without an epsilon that could hide scoring differences.
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
                let actual = a.get(key).unwrap_or_else(|| panic!("{path}.{key} missing"));
                assert_semantically_equal(actual, value, &format!("{path}.{key}"));
            }
        }
        _ => assert_eq!(actual, expected, "{path}"),
    }
}

fn check_case(input: &Value, expected: &Value, aliases: &[String]) {
    let context = &input["context"];
    let season = Season(context["season"].as_u64().unwrap().try_into().unwrap());
    let mut history = PlayerTeamHistory::new(season);
    for (player, team) in context["initial_history"].as_object().unwrap() {
        history.insert(
            PlayerId(player.parse().unwrap()),
            ProTeamId(team.as_u64().unwrap().try_into().unwrap()),
        );
    }
    let scores = WeeklyBoxScores::from_values(
        &input["box_scores"],
        &input["pro_schedule"],
        &input["positional_ratings"],
        BoxScoreContext {
            season,
            scoring_period: ScoringPeriod(
                context["scoring_period"]
                    .as_u64()
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
            matchup_period: MatchupPeriod(
                context["matchup_period"]
                    .as_u64()
                    .unwrap()
                    .try_into()
                    .unwrap(),
            ),
            now_unix_ms: context["now_unix_ms"].as_i64().unwrap(),
            history: &mut history,
        },
    )
    .unwrap();
    assert_semantically_equal(
        &output_projection(&scores, &history, aliases),
        expected,
        input["name"].as_str().unwrap(),
    );
}

fn excluded_aliases() -> Vec<String> {
    serde_json::from_value(
        read_json(fixture_dir().join("provenance.json"))["excluded_duplicate_stat_labels"].clone(),
    )
    .unwrap()
}

#[test]
fn synthetic_modern_weekly_lineups_match_frozen_python_oracle() {
    let input = read_json(fixture_dir().join("synthetic_2024_input.json"));
    let expected = read_json(fixture_dir().join("synthetic_2024_expected.json"));
    let cases = input["cases"].as_array().unwrap();
    let goldens = expected["cases"].as_array().unwrap();
    assert_eq!(cases.len(), goldens.len());
    let aliases = excluded_aliases();
    for (case, golden) in cases.iter().zip(goldens) {
        assert_eq!(case["name"], golden["name"]);
        check_case(case, &golden["output"], &aliases);
    }
}

#[test]
fn historical_2018_selected_model_payload_matches_python_oracle() {
    // This bypasses the supported-season HTTP gate deliberately: the original
    // file is 2018, not evidence of a fresh modern/private-league response.
    check_case(
        &read_json(fixture_dir().join("historical_2018_input.json")),
        &read_json(fixture_dir().join("historical_2018_expected.json")),
        &excluded_aliases(),
    );
}

#[test]
fn full_historical_source_matches_selected_projection_when_available() {
    let Some(reference) = std::env::var_os("ESPN_PYTHON_REFERENCE") else {
        return;
    };
    let source =
        read_json(Path::new(&reference).join("tests/football/unit/data/league_boxscore_2018.json"));
    let mut input = read_json(fixture_dir().join("historical_2018_input.json"));
    input["box_scores"] = json!({"schedule": [source["schedule"][0]]});
    check_case(
        &input,
        &read_json(fixture_dir().join("historical_2018_expected.json")),
        &excluded_aliases(),
    );
}
