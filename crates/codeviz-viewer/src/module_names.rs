//! What each module sphere is called: its own name, or where two modules share one,
//! as much of the path before it as tells them apart, a word at a time
//! (`layout::graph` and `buildkite::graph`, `codeviz::diff` and `core::diff`).

/// Each module's name, by position in `modules`, given its id (`rust:codeviz_layout::graph`)
/// and its own name (`graph`); `None` for a module that has none.
pub(crate) fn qualified(modules: &[Option<(&str, &str)>]) -> Vec<String> {
    // The path without its scheme, and where each word of it starts, up to the name.
    let paths: Vec<Option<(&str, Vec<usize>)>> = modules
        .iter()
        .map(|module| {
            let (id, name) = (*module)?;
            let path = match id.split_once(':') {
                Some((scheme, rest)) if !scheme.is_empty() && !rest.starts_with(':') => rest,
                _ => id,
            };
            let name_at = path.strip_suffix(name).map_or(path.len(), str::len);
            let mut starts = Vec::new();
            let mut before = None;
            for (at, c) in path.char_indices().take_while(|&(at, _)| at < name_at) {
                if c.is_alphanumeric() && !before.is_some_and(char::is_alphanumeric) {
                    starts.push(at);
                }
                before = Some(c);
            }
            Some((path, starts))
        })
        .collect();
    let mut names: Vec<String> = modules
        .iter()
        .map(|m| m.map_or_else(String::new, |(_, name)| name.to_owned()))
        .collect();
    // How many words of the path before its name each one shows.
    let mut words = vec![0; modules.len()];
    loop {
        let mut grew = false;
        for one in 0..names.len() {
            if names[one].is_empty() {
                continue;
            }
            let shared = (0..names.len())
                .any(|other| other != one && names[other].eq_ignore_ascii_case(&names[one]));
            let Some((_, starts)) = paths[one].as_ref().filter(|_| shared) else {
                continue;
            };
            if words[one] < starts.len() {
                words[one] += 1;
                grew = true;
            }
        }
        if !grew {
            break;
        }
        for (one, name) in names.iter_mut().enumerate() {
            if let (Some((path, starts)), Some(own)) = (&paths[one], modules[one]) {
                *name = match words[one] {
                    0 => own.1.to_owned(),
                    n => path[starts[starts.len() - n]..].to_owned(),
                };
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_two_modules_share_takes_the_words_that_tell_them_apart() {
        let names = qualified(&[
            Some(("rust:codeviz_layout::graph", "graph")),
            Some(("rust:codeviz_buildkite::graph", "graph")),
            Some(("rust:codeviz::diff", "diff")),
            Some(("rust:codeviz_core::diff", "diff")),
            Some(("rust:codeviz_viewer::ui", "ui")),
            None,
        ]);
        assert_eq!(
            names,
            [
                "layout::graph",
                "buildkite::graph",
                "codeviz::diff",
                "core::diff",
                "ui",
                ""
            ]
        );
    }

    #[test]
    fn three_alike_grow_until_each_is_its_own() {
        let names = qualified(&[
            Some(("rust:a_x::m", "m")),
            Some(("rust:b_x::m", "m")),
            Some(("rust:c_y::m", "m")),
        ]);
        assert_eq!(names, ["a_x::m", "b_x::m", "y::m"]);
    }
}
