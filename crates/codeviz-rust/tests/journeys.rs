//! Golden journeys: every `pub fn` in `fixtures/journeys` is unrolled and compared with
//! `fixtures/journeys/expected/<name>.txt`. Run with `CODEVIZ_BLESS=1` to rewrite them,
//! then review the diff against the fixture source.

#![allow(clippy::unwrap_used, reason = "test helpers fail loudly")]

use std::{fmt::Write as _, fs, path::Path};

use codeviz_core::{Journey, JourneyOptions, System, Zoom, describe_scenario, outline, scenarios};
use codeviz_ir::UnitId;
use codeviz_rust::{AnalyzeOptions, analyze};

const ENTRIES: [&str; 10] = [
    "awaited",
    "evaluation_order",
    "propagation",
    "branching",
    "looping",
    "closures",
    "asynchronous",
    "dispatch",
    "recursion",
    "fields",
];

fn render(system: &System, entry: &UnitId) -> String {
    let mut out = String::from("scenarios:\n");
    let found = scenarios(system, entry, &Zoom::default());
    for (index, scenario) in found.iter().enumerate() {
        let _ = writeln!(out, "  [{index}] {}", describe_scenario(system, scenario));
    }
    let journey = Journey::unroll(system, entry, None, JourneyOptions::default()).unwrap();
    out.push_str("\njourney:\n");
    out.push_str(&outline(system, &journey));
    for (index, scenario) in found.iter().enumerate().skip(1) {
        let journey =
            Journey::unroll(system, entry, Some(scenario), JourneyOptions::default()).unwrap();
        let _ = write!(out, "\nscenario [{index}]:\n{}", outline(system, &journey));
    }
    out
}

#[test]
fn golden_journeys() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/journeys");
    let document = analyze(&fixture, AnalyzeOptions::default())
        .unwrap()
        .document;
    assert_eq!(codeviz_ir::validate(&document), Vec::new());
    let system = System::new([document]);
    let bless = std::env::var_os("CODEVIZ_BLESS").is_some();

    let mut mismatches = Vec::new();
    for entry in ENTRIES {
        let actual = render(&system, &UnitId::new(format!("rust:journeys::{entry}")));
        let path = fixture.join("expected").join(format!("{entry}.txt"));
        if bless {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, &actual).unwrap();
        } else if fs::read_to_string(&path).ok().as_deref() != Some(actual.as_str()) {
            mismatches.push(format!("--- {entry} ---\n{actual}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "journeys differ from expected (CODEVIZ_BLESS=1 to update):\n{}",
        mismatches.join("\n")
    );
}
