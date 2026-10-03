//! Offline Python football player oracle: captured 2019 card and synthetic modern
//! free-agent/card inputs. Duplicate-name behavior is an explicit Rust extension.
use espn_fantasy_football::{
    Player, ScoringPeriod, Season, StatId,
    football::{
        BoxPlayer, FreeAgentContext, FreeAgentPage, PlayerCard, PlayerDirectory,
        pro_team_abbreviation, slot_label, stat_label,
    },
};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, fs, path::Path};

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/players")
}
fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}
fn aliases() -> Vec<String> {
    serde_json::from_value(
        read_json(fixture_dir().join("provenance.json"))["excluded_duplicate_stat_labels"].clone(),
    )
    .unwrap()
}
fn breakdown(data: &BTreeMap<StatId, f64>, aliases: &[String]) -> Value {
    let mut fields = Map::new();
    for (id, number) in data {
        let label = stat_label(*id)
            .map(str::to_owned)
            .unwrap_or_else(|| id.to_string());
        if !aliases.contains(&label) {
            fields.insert(label, json!(number));
        }
    }
    Value::Object(fields)
}
fn player_projection(player: &Player, aliases: &[String]) -> Value {
    let mut stats = Map::new();
    for (period, lines) in &player.stats {
        let mut fields = Map::new();
        for (name, line) in [("actual", &lines.actual), ("projected", &lines.projected)] {
            if let Some(line) = line {
                fields.insert(
                    name.into(),
                    json!({
                        "points": line.points, "average_points": line.average_points,
                        "breakdown": breakdown(&line.breakdown, aliases),
                        "points_breakdown": breakdown(&line.points_breakdown, aliases),
                    }),
                );
            }
        }
        stats.insert(period.to_string(), Value::Object(fields));
    }
    json!({
        "id": player.id, "name": player.name,
        "position": player.position.and_then(slot_label),
        "eligible_slots": player.eligible_slots.iter().map(|id| slot_label(*id)).collect::<Vec<_>>(),
        "pro_team": pro_team_abbreviation(player.pro_team),
        "positional_rank": player.positional_rank, "jersey": player.jersey,
        "acquisition_type": player.acquisition_type, "on_team_id": player.on_team_id,
        "lineup_slot": player.lineup_slot.and_then(slot_label),
        "injury_status": player.injury_status, "injured": player.injured,
        "percent_owned": player.percent_owned, "percent_started": player.percent_started,
        "active_status": player.active_status, "total_points": player.total_points,
        "projected_total_points": player.projected_total_points,
        "avg_points": player.avg_points, "projected_avg_points": player.projected_avg_points,
        "stats": stats,
    })
}
fn free_agent_projection(player: &BoxPlayer, aliases: &[String]) -> Value {
    json!({
        "player": player_projection(&player.player, aliases),
        "slot_position": player.slot_position_label(),
        "pro_team": pro_team_abbreviation(player.pro_team),
        "pro_opponent": player.pro_opponent.and_then(pro_team_abbreviation),
        "pro_pos_rank": player.pro_pos_rank, "game_date_unix_ms": player.game_date_unix_ms,
        "game_played": player.game_played, "on_bye_week": player.on_bye_week,
        "points": player.points, "projected_points": player.projected_points,
    })
}
fn card_projection(card: &PlayerCard, aliases: &[String]) -> Value {
    let schedule: Map<String, Value> = card.schedule.iter().map(|(period, game)| (period.to_string(), json!({
        "opponent": pro_team_abbreviation(game.opponent), "date_unix_ms": game.date_unix_ms,
    }))).collect();
    let transactions = card
        .raw
        .get("transactions")
        .or_else(|| card.raw.get("player").and_then(|p| p.get("transactions")));
    json!({
        "player": player_projection(&card.player, aliases),
        "schedule": schedule,
        "raw_transactions": transactions.cloned().unwrap_or_else(|| json!([])),
    })
}
/// Exact numeric equality, allowing Python integer zero versus Rust float zero.
/// No blanket floating-point tolerance conceals rounding/statistics differences.
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
fn check_cards(input: &Value, expected: &Value) {
    let cards = PlayerCard::from_values(
        &input["cards"],
        &input["pro_schedule"],
        Season(input["season"].as_u64().unwrap().try_into().unwrap()),
    )
    .unwrap();
    let projected = json!(
        cards
            .iter()
            .map(|card| card_projection(card, &aliases()))
            .collect::<Vec<_>>()
    );
    assert_semantically_equal(&projected, expected, "cards");
    for (card, raw) in cards
        .iter()
        .zip(input["cards"]["players"].as_array().unwrap())
    {
        assert_eq!(
            &card.raw, raw,
            "Raw wrapper retains all unimplemented metadata verbatim"
        );
    }
}

#[test]
fn free_agents_preserve_python_order_statistics_and_weekly_enrichment() {
    let inputs = read_json(fixture_dir().join("free_agents_input.json"));
    let goldens = read_json(fixture_dir().join("free_agents_expected.json"));
    assert_eq!(
        inputs["cases"].as_array().unwrap().len(),
        goldens["cases"].as_array().unwrap().len()
    );
    for (input, golden) in inputs["cases"]
        .as_array()
        .unwrap()
        .iter()
        .zip(goldens["cases"].as_array().unwrap())
    {
        assert_eq!(input["name"], golden["name"]);
        let context = &input["context"];
        let page = FreeAgentPage::from_values(
            &input["players"],
            &input["pro_schedule"],
            &input["positional_ratings"],
            FreeAgentContext {
                season: Season(context["season"].as_u64().unwrap().try_into().unwrap()),
                scoring_period: ScoringPeriod(
                    context["scoring_period"]
                        .as_u64()
                        .unwrap()
                        .try_into()
                        .unwrap(),
                ),
                now_unix_ms: context["now_unix_ms"].as_i64().unwrap(),
                offset: context["offset"].as_u64().unwrap().try_into().unwrap(),
                limit: context["limit"].as_u64().unwrap().try_into().unwrap(),
            },
        )
        .unwrap();
        let projection = json!(
            page.players
                .iter()
                .map(|p| free_agent_projection(p, &aliases()))
                .collect::<Vec<_>>()
        );
        assert_semantically_equal(
            &projection,
            &golden["players"],
            input["name"].as_str().unwrap(),
        );
    }
}

#[test]
fn captured_2019_and_synthetic_modern_cards_match_python_projection() {
    for prefix in ["historical_2019", "synthetic_2024"] {
        check_cards(
            &read_json(fixture_dir().join(format!("{prefix}_cards_input.json"))),
            &read_json(fixture_dir().join(format!("{prefix}_cards_expected.json"))),
        );
    }
}

#[test]
fn directory_retains_duplicate_name_ids_as_explicit_rust_extension() {
    let fixture = read_json(fixture_dir().join("directory.json"));
    let directory = PlayerDirectory::from_value(&fixture["input"], Season(2024)).unwrap();
    assert_eq!(json!(directory.players), fixture["expected"]);
    for (name, expected) in fixture["queries"].as_object().unwrap() {
        assert_eq!(json!(directory.ids_named(name)), *expected);
    }
    let traces = read_json(fixture_dir().join("python_requests.json"));
    let first_id = traces["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "player_directory")
        .unwrap()["python_first_name_id"]
        .as_i64()
        .unwrap();
    assert_eq!(directory.ids_named("Alex Doe")[0].0, first_id);
    assert_eq!(directory.ids_named("Alex Doe").len(), 2);
}

#[test]
fn full_original_card_matches_same_projection_when_available() {
    let Some(reference) = std::env::var_os("ESPN_PYTHON_REFERENCE") else {
        return;
    };
    let mut input = read_json(fixture_dir().join("historical_2019_cards_input.json"));
    input["cards"] = read_json(
        Path::new(&reference).join("tests/football/unit/data/league_2019_playerCard.json"),
    );
    check_cards(
        &input,
        &read_json(fixture_dir().join("historical_2019_cards_expected.json")),
    );
}
