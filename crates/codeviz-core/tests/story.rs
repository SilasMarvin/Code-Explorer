//! Journeys told as Markdown and Mermaid, over the demo documents.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::collections::HashMap;

use codeviz_core::{
    Diff, Journey, JourneyOptions, Story, System, Zoom, align, number_calls, scenarios,
};
use codeviz_ir::{Document, UnitId};

const DEMOS: [&str; 4] = [
    "axum-server",
    "axum-server-next",
    "python-service",
    "deploy-pipeline",
];

fn document(name: &str) -> Document {
    let path = format!("{}/../../demo/out/{name}.json", env!("CARGO_MANIFEST_DIR"));
    Document::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn demo(name: &str) -> System {
    System::new([document(name)])
}

/// The story of `entry` (an entry label), in scenario `scenario`, `depth` levels deep.
fn tell(system: &System, entry: &str, scenario: usize, depth: u32) -> Story {
    let entry = system.find(entry)[0].id.clone();
    let scenarios = scenarios(system, &entry, &Zoom::default());
    let journey = Journey::unroll(
        system,
        &entry,
        scenarios.get(scenario),
        JourneyOptions::default(),
    )
    .unwrap();
    let numbers = number_calls(system, &journey, depth);
    Story::new(system, &journey, &scenarios, Some(scenario), &numbers)
}

/// Every entry of every demo, in every scenario, at a few depths.
fn every_story() -> Vec<(String, Story)> {
    let mut stories = Vec::new();
    for name in DEMOS {
        let system = demo(name);
        for entry in system.entries() {
            let count = scenarios(&system, &entry.unit, &Zoom::default()).len();
            for scenario in 0..count {
                for depth in [1, 2, 4, u32::MAX] {
                    let story = tell(&system, &entry.label, scenario, depth);
                    let what = format!("{name} {} [{scenario}] depth {depth}", entry.label);
                    stories.push((what, story));
                }
            }
        }
    }
    stories
}

#[test]
fn tells_the_happy_path_with_where_it_can_fail() {
    let system = demo("axum-server");
    let markdown = tell(&system, "POST /users/{id}/orders", 0, u32::MAX).markdown();
    assert_eq!(
        markdown,
        "\
`POST /users/{id}/orders` → `handlers.place_order`, when it completes:

- 1 `orders.place`
  - 1.1 `users.find_user` — can fail: `.ok_or` fails
    - 1.1.1 `Store.user`
  - 1.2 `validation.check_quantity` — can fail: `AppError::Invalid(format!(\"quantity must be between 1 and {MAX_QUANTITY}\"))`
  - 1.3 `pricing.quote`
    - 1.3.1 `pricing.unit_price` — can fail: `AppError::Invalid(format!(\"unknown sku {other}\"))`
    - 1.3.2 `pricing.apply_discount`
      - 1.3.2.1 `pricing.bulk_discount` — only if `quantity >= BULK_THRESHOLD`
  - 1.4 `Store.insert_order`
    - 1.4.1 `Store.next_id`

Outcomes:
- completes (this path)
- error `.ok_or` fails in `users.find_user`
- error `AppError::Invalid(format!(\"quantity must be between 1 and {MAX_QUANTITY}\"))` in `validation.check_quantity`
- error `AppError::Invalid(format!(\"unknown sku {other}\"))` in `pricing.unit_price`
"
    );
}

#[test]
fn a_shallow_story_folds_calls_and_lifts_failures_to_what_shows() {
    let system = demo("axum-server");
    let markdown = tell(&system, "POST /users/{id}/orders", 0, 1).markdown();
    let calls: Vec<&str> = markdown.lines().filter(|l| l.starts_with("- 1")).collect();
    assert_eq!(calls.len(), 1, "{markdown}");
    assert!(
        calls[0].starts_with(
            "- 1 `orders.place` — can fail in `users.find_user`, `validation.check_quantity`, \
             `pricing.unit_price`, +4 calls"
        ),
        "{markdown}"
    );
}

#[test]
fn a_failing_scenario_ends_where_it_fails() {
    let system = demo("axum-server");
    let story = tell(&system, "POST /users/{id}/orders", 3, u32::MAX);
    let markdown = story.markdown();
    assert!(
        markdown.starts_with(
            "`POST /users/{id}/orders` → `handlers.place_order`, when it ends in error \
             `AppError::Invalid(format!(\"unknown sku {other}\"))` in `pricing.unit_price`:"
        ),
        "{markdown}"
    );
    let last = story.beats().last().unwrap();
    assert_eq!(last.name, "pricing.unit_price");
    assert!(last.ends.is_some(), "the path ends at the exit");
    assert!(
        story.beats().all(|b| b.fails.is_empty()),
        "a failing path does not list other failures"
    );
}

#[test]
fn numbers_follow_the_outline_in_every_demo() {
    for (what, story) in every_story() {
        let markdown = story.markdown();
        // Each call is a list item, indented by its depth, led by its number.
        let listed: Vec<String> = markdown
            .lines()
            .take_while(|line| *line != "Outcomes:")
            .filter_map(|line| {
                let item = line.trim_start().strip_prefix("- ")?;
                let (number, _) = item.split_once(" `")?;
                let numbered = number.chars().all(|c| c.is_ascii_digit() || c == '.');
                numbered
                    .then(|| format!("{}{number}", &line[..line.len() - line.trim_start().len()]))
            })
            .collect();
        let expected: Vec<String> = story
            .beats()
            .map(|b| format!("{}{}", "  ".repeat(b.number.matches('.').count()), b.number))
            .collect();
        assert_eq!(listed, expected, "{what}:\n{markdown}");
    }
}

#[test]
fn shows_conditions_and_handoffs_in_python() {
    let system = demo("python-service");
    let markdown = tell(&system, "POST /users/{user_id}/orders/bulk", 0, u32::MAX).markdown();
    assert!(markdown.contains("— repeats"), "{markdown}");
    assert!(
        markdown.contains(
            "— only if not `quantity >= BULK_THRESHOLD * 10` and `quantity >= BULK_THRESHOLD`"
        ),
        "{markdown}"
    );
    assert!(markdown.contains("in the background"), "{markdown}");
}

/// What a diagram line is, read the way Mermaid reads a sequence diagram.
#[derive(Debug, PartialEq)]
enum Line<'a> {
    Participant(&'a str),
    Message(&'a str, &'a str, &'a str),
    Activate(&'a str),
    Deactivate(&'a str),
    Open(&'a str),
    Else,
    End,
}

fn parse(line: &str) -> Option<Line<'_>> {
    let line = line.trim();
    let word = line.split(' ').next()?;
    match word {
        "participant" | "actor" => {
            let rest = line.strip_prefix(word)?.trim();
            let id = rest.split_once(" as ").map_or(rest, |(id, _)| id);
            let plain = id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            (plain && !id.is_empty()).then_some(Line::Participant(id))
        }
        "activate" => Some(Line::Activate(line.strip_prefix("activate ")?)),
        "deactivate" => Some(Line::Deactivate(line.strip_prefix("deactivate ")?)),
        "opt" | "alt" | "loop" => Some(Line::Open(word)),
        "else" => Some(Line::Else),
        "end" => Some(Line::End),
        _ => {
            let (head, text) = line.split_once(": ")?;
            let arrow = ["--x", "->>", "-)"]
                .into_iter()
                .find(|a| head.contains(a))?;
            let (from, to) = head.split_once(arrow)?;
            let id =
                |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            (id(from) && id(to) && !text.is_empty()).then_some(Line::Message(from, arrow, to))
        }
    }
}

/// A line with its `#59;`-style entities taken out.
fn strip_entities(line: &str) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(at) = rest.find('#') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let digits = after.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && after[digits..].starts_with(';') {
            rest = &after[digits + 1..];
        } else {
            out.push('#');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// Checks a diagram the way Mermaid would reject it: unknown lines, messages between
/// undeclared participants, unbalanced blocks or activations, `else` outside `alt`,
/// and characters it reads as syntax inside text.
fn check(diagram: &str) -> Result<(), String> {
    let mut lines = diagram.lines();
    if lines.next() != Some("sequenceDiagram") {
        return Err("does not start with sequenceDiagram".to_owned());
    }
    let mut participants = Vec::new();
    let mut blocks: Vec<&str> = Vec::new();
    let mut active: HashMap<&str, usize> = HashMap::new();
    for line in lines {
        let parsed = parse(line).ok_or_else(|| format!("cannot read `{line}`"))?;
        let bare = strip_entities(line);
        if bare.contains(';') || bare.contains('#') || bare.contains("%%") {
            return Err(format!("unescaped syntax in `{line}`"));
        }
        match parsed {
            Line::Participant(id) => {
                if participants.contains(&id) {
                    return Err(format!("{id} declared twice"));
                }
                participants.push(id);
            }
            Line::Message(from, _, to) => {
                for id in [from, to] {
                    if !participants.contains(&id) {
                        return Err(format!("undeclared participant {id} in `{line}`"));
                    }
                }
            }
            Line::Activate(id) => *active.entry(id).or_default() += 1,
            Line::Deactivate(id) => {
                let count = active.entry(id).or_default();
                if *count == 0 {
                    return Err(format!("{id} deactivated while inactive"));
                }
                *count -= 1;
            }
            Line::Open(kind) => blocks.push(kind),
            Line::Else if blocks.last() == Some(&"alt") => {}
            Line::Else => return Err(format!("else outside alt: {blocks:?}")),
            Line::End => {
                blocks.pop().ok_or("end without a block")?;
            }
        }
    }
    if !blocks.is_empty() {
        return Err(format!("unclosed blocks {blocks:?}"));
    }
    if active.values().any(|&count| count > 0) {
        return Err("activations left open".to_owned());
    }
    Ok(())
}

#[test]
fn every_demo_diagram_is_well_formed() {
    for (what, story) in every_story() {
        let diagram = story.mermaid();
        if let Err(problem) = check(&diagram) {
            panic!("{what}: {problem}\n{diagram}");
        }
        let messages = diagram
            .lines()
            .filter(|l| matches!(parse(l), Some(Line::Message(_, "->>" | "-)", _))))
            .count();
        let entry = usize::from(story.entry.is_some());
        assert_eq!(
            messages,
            story.beats().count() + entry,
            "{what}: one message per call\n{diagram}"
        );
    }
}

#[test]
fn arms_of_one_branch_share_an_alt() {
    let system = demo("python-service");
    let diagram = tell(&system, "POST /users/{user_id}/orders/bulk", 0, u32::MAX).mermaid();
    let at = diagram
        .find("alt if quantity #62;= BULK_THRESHOLD * 10")
        .unwrap();
    let rest = &diagram[at..];
    let else_at = rest.find("else otherwise").unwrap();
    assert!(rest[..else_at].contains("bulk_discount"), "{diagram}");
    assert!(
        rest[else_at..].contains("opt if quantity #62;= BULK_THRESHOLD"),
        "{diagram}"
    );
}

#[test]
fn the_checker_rejects_what_mermaid_would() {
    assert!(check("sequenceDiagram\n    participant a as A\n    a->>b: hi").is_err());
    assert!(check("sequenceDiagram\n    participant a as A\n    opt x\n    a->>a: hi").is_err());
    assert!(
        check("sequenceDiagram\n    participant a as A\n    opt x\n    else y\n    end").is_err()
    );
    assert!(check("sequenceDiagram\n    participant a as A\n    a->>a: a; b").is_err());
    assert!(check("sequenceDiagram\n    participant a as A\n    deactivate a").is_err());
    assert!(
        check(
            "sequenceDiagram\n    participant a as A\n    alt x\n    a->>a: hi\n    else y\n    end"
        )
        .is_ok()
    );
}

#[test]
fn summarizes_a_diff_by_module_with_the_entries_it_reaches() {
    let after = demo("axum-server-next");
    let (before, moved) = align(vec![document("axum-server")], &after);
    let before = System::new(before);
    let diff = Diff::between(&before, &after).with_moves(moved);
    let markdown = diff.markdown(&before, &after);
    assert!(
        markdown.starts_with(
            "11 functions changed (5 new, 5 rewritten, 1 removed); 5 calls added, 1 removed."
        ),
        "{markdown}"
    );
    assert!(markdown.contains("  - new module `audit`\n"), "{markdown}");
    assert!(
        markdown.contains("\n- `axum_server::services::pricing`\n  - removed `bulk_discount`\n"),
        "{markdown}"
    );
    assert!(
        markdown.contains("\nChanged functions:\n- `axum_server::"),
        "{markdown}"
    );
    assert!(
        markdown.contains("\nEntry points affected:\n"),
        "{markdown}"
    );
    assert!(
        markdown.contains(
            "- `POST /users/{id}/orders` (`handlers.place_order`) reaches `orders.place`, \
             `audit.record`, `pricing.quote`, and 7 more"
        ),
        "{markdown}"
    );
    let changed: Vec<&UnitId> = diff.units.keys().collect();
    for id in changed {
        let name = after
            .unit(id)
            .or_else(|| before.unit(id))
            .unwrap()
            .name
            .clone();
        assert!(
            markdown.contains(&format!("`{name}`")),
            "{name} missing:\n{markdown}"
        );
    }
}

#[test]
fn a_journey_is_told_as_deep_as_fits() {
    let system = demo("axum-server");
    let entry = system.find("POST /users/{id}/orders")[0].id.clone();
    let journey = Journey::unroll(&system, &entry, None, JourneyOptions::default()).unwrap();
    assert_eq!(
        codeviz_core::depth_to_fit(&system, &journey, 100),
        4,
        "all of it"
    );
    assert_eq!(
        codeviz_core::depth_to_fit(&system, &journey, 5),
        2,
        "1 + 4 calls"
    );
    assert_eq!(
        codeviz_core::depth_to_fit(&system, &journey, 0),
        1,
        "at least one level"
    );
}
