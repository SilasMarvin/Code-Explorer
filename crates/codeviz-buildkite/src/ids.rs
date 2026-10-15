//! Step ids.
//!
//! A step's id is its `key`. Steps without one get an id derived from what a reader
//! sees: the label with emoji and `{{…}}` interpolations removed, else the first
//! command or the triggered pipeline, else the step type. Duplicates take `-2`, `-3`,
//! … in pipeline order. Declared keys are reserved first, so a derived id never takes
//! a key written elsewhere in the file, and adding a step only renames later steps
//! that share its derived name.

use std::collections::HashSet;

use crate::{
    Error,
    model::{Kind, Step},
};

pub(crate) fn assign(steps: &mut [Step]) -> Result<(), Error> {
    let mut taken = HashSet::new();
    reserve(steps, &mut taken)?;
    derive(steps, &mut taken);
    Ok(())
}

fn reserve(steps: &mut [Step], taken: &mut HashSet<String>) -> Result<(), Error> {
    for step in steps {
        if let Some(key) = &step.key {
            if key.is_empty() || key.contains(char::is_whitespace) {
                return Err(Error::InvalidKey(key.clone()));
            }
            if !taken.insert(key.clone()) {
                return Err(Error::DuplicateKey(key.clone()));
            }
            step.id.clone_from(key);
        }
        if let Kind::Group(children) = &mut step.kind {
            reserve(children, taken)?;
        }
    }
    Ok(())
}

fn derive(steps: &mut [Step], taken: &mut HashSet<String>) {
    for step in steps {
        if step.id.is_empty() {
            let base = base_name(step);
            let mut id = base.clone();
            let mut n = 2;
            while taken.contains(&id) {
                id = format!("{base}-{n}");
                n += 1;
            }
            taken.insert(id.clone());
            step.id = id;
        }
        if let Kind::Group(children) = &mut step.kind {
            derive(children, taken);
        }
    }
}

fn base_name(step: &Step) -> String {
    let hint = match &step.kind {
        Kind::Command(command) => command.lines.first().map(String::as_str),
        Kind::Trigger { pipeline, .. } => Some(pipeline.as_str()),
        _ => None,
    };
    step.label
        .as_deref()
        .into_iter()
        .chain(hint)
        .map(slug)
        .find(|slug| !slug.is_empty())
        .unwrap_or_else(|| step.kind.name().to_owned())
}

/// Lowercase ASCII words joined by `-`, ignoring `:emoji:` and `{{interpolation}}`.
pub(crate) fn slug(text: &str) -> String {
    let text = display_name(text);
    let mut slug = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// The label as shown to people: `:emoji:` shortcodes and `{{…}}` removed.
pub(crate) fn display_name(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut rest = label;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("{{")
            && let Some(end) = after.find("}}")
        {
            rest = &after[end + 2..];
        } else if let Some(after) = rest.strip_prefix(':')
            && let Some(end) = after.find(':')
            && is_shortcode(&after[..end])
        {
            rest = &after[end + 1..];
        } else {
            let mut chars = rest.chars();
            out.extend(chars.next());
            rest = chars.as_str();
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_shortcode(name: &str) -> bool {
    name.chars().any(|c| c.is_ascii_alphabetic())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::parse;

    fn ids(yaml: &str) -> Vec<String> {
        let mut pipeline = parse(yaml).unwrap();
        assign(&mut pipeline.steps).unwrap();
        let mut out = Vec::new();
        collect(&pipeline.steps, &mut out);
        out
    }

    fn collect(steps: &[Step], out: &mut Vec<String>) {
        for step in steps {
            out.push(step.id.clone());
            if let Kind::Group(children) = &step.kind {
                collect(children, out);
            }
        }
    }

    #[test]
    fn slugs_ignore_emoji_and_interpolation() {
        assert_eq!(slug(":rust: Test {{matrix.os}} (stable)"), "test-stable");
        assert_eq!(
            slug("cargo audit --deny warnings"),
            "cargo-audit-deny-warnings"
        );
        assert_eq!(display_name(":docker: Build  image"), "Build image");
        assert_eq!(display_name("ratio 1:2:3"), "ratio 1:2:3");
    }

    #[test]
    fn keys_win_and_derived_ids_are_deduplicated() {
        let yaml = "
steps:
  - {label: Lint, command: make}
  - wait
  - {label: ':rust: Lint', command: make}
  - key: lint
    command: cargo clippy
  - command: cargo audit
  - wait
  - group: Deploy
    steps:
      - block: Ship?
";
        assert_eq!(
            ids(yaml),
            [
                "lint-2",
                "wait",
                "lint-3",
                "lint",
                "cargo-audit",
                "wait-2",
                "deploy",
                "ship"
            ]
        );
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let mut pipeline =
            parse("steps:\n  - {key: a, command: make}\n  - {key: a, command: make}\n").unwrap();
        assert!(matches!(
            assign(&mut pipeline.steps),
            Err(Error::DuplicateKey(key)) if key == "a"
        ));
    }
}
