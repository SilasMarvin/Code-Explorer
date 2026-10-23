//! The last view of each set of documents, kept by the viewer as the flags that open
//! it, for `--resume`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Where the last view of what `key` names is kept: under `$XDG_STATE_HOME/codeviz`,
/// else `~/Library/Application Support/codeviz` on a Mac, else
/// `~/.local/state/codeviz`. `name` makes the file findable by eye.
pub(crate) fn file(key: &str, name: &str) -> Option<PathBuf> {
    let dir = dir(
        std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        cfg!(target_os = "macos"),
    )?;
    let name: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    Some(
        dir.join("views")
            .join(format!("{name}-{:016x}.json", fnv(key))),
    )
}

fn dir(xdg_state: Option<PathBuf>, home: Option<PathBuf>, mac: bool) -> Option<PathBuf> {
    let xdg = xdg_state.filter(|d| d.is_absolute());
    match (xdg, home) {
        (Some(state), _) => Some(state.join("codeviz")),
        (None, Some(home)) if mac => Some(home.join("Library/Application Support/codeviz")),
        (None, Some(home)) => Some(home.join(".local/state/codeviz")),
        (None, None) => None,
    }
}

/// A key for a set of files: their absolute paths, in order.
pub(crate) fn key<'a>(parts: impl IntoIterator<Item = &'a Path>) -> String {
    parts
        .into_iter()
        .map(|p| {
            p.canonicalize()
                .unwrap_or_else(|_| p.to_path_buf())
                .display()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The flags kept as the last view in `path`, if any.
pub(crate) fn saved_flags(path: &Path) -> Option<Vec<String>> {
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    json.get("flags")?
        .as_array()?
        .iter()
        .map(|f| f.as_str().map(str::to_owned))
        .collect()
}

/// Flags that say where a view stands, each with its value.
const PLACE: [&str; 5] = ["--focus", "--step", "--scenario", "--spotlight", "--at"];

/// The command line `args` with `saved` inserted after the subcommand, so that any
/// flag given again wins. When `args` says where to stand, `saved` does not.
pub(crate) fn resume(args: &[OsString], saved: &[String]) -> Vec<OsString> {
    let placed = args
        .iter()
        .any(|a| ["--focus", "--at", "--entry", "-e"].contains(&a.to_string_lossy().as_ref()));
    let mut kept = Vec::new();
    let mut flags = saved.iter();
    while let Some(flag) = flags.next() {
        if placed && PLACE.contains(&flag.as_str()) {
            flags.next();
            continue;
        }
        kept.push(OsString::from(flag));
    }
    let mut resumed = args.to_vec();
    // Program, then subcommand.
    let at = 2.min(resumed.len());
    resumed.splice(at..at, kept);
    resumed
}

/// FNV-1a: a hash that stays the same from one build to the next.
pub(crate) fn fnv(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(words: &[&str]) -> Vec<OsString> {
        words.iter().map(OsString::from).collect()
    }

    #[test]
    fn keeps_views_where_the_system_keeps_state() {
        let home = Some(PathBuf::from("/home/me"));
        assert_eq!(
            dir(None, home.clone(), true),
            Some(PathBuf::from(
                "/home/me/Library/Application Support/codeviz"
            ))
        );
        assert_eq!(
            dir(None, home.clone(), false),
            Some(PathBuf::from("/home/me/.local/state/codeviz"))
        );
        assert_eq!(
            dir(Some("/state".into()), home.clone(), true),
            Some(PathBuf::from("/state/codeviz"))
        );
        assert_eq!(
            dir(Some("relative".into()), home, false),
            Some(PathBuf::from("/home/me/.local/state/codeviz")),
            "a relative XDG path is ignored, as the spec says"
        );
    }

    #[test]
    fn resumes_with_what_is_given_winning() {
        let saved: Vec<String> = ["--focus", "orders::place", "--step", "3", "--depth", "2"]
            .map(str::to_owned)
            .into();
        assert_eq!(
            resume(
                &os(&["codeviz", "view", "a.json", "--resume", "--depth", "4"]),
                &saved
            ),
            os(&[
                "codeviz",
                "view",
                "--focus",
                "orders::place",
                "--step",
                "3",
                "--depth",
                "2",
                "a.json",
                "--resume",
                "--depth",
                "4"
            ])
        );
        assert_eq!(
            resume(
                &os(&["codeviz", "view", "a.json", "--resume", "--at", "x.rs:3"]),
                &saved
            ),
            os(&[
                "codeviz", "view", "--depth", "2", "a.json", "--resume", "--at", "x.rs:3"
            ]),
            "a new place drops the old one"
        );
    }

    #[test]
    fn hashes_the_same_every_time() {
        assert_eq!(fnv(""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv("a"), 0xaf63_dc4c_8601_ec8c);
    }
}
