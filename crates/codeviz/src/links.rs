//! Links from a told journey to each function's source on the web: under a base the
//! user gives, or the GitHub commit the analyzed code's repository is at.

use std::{collections::HashMap, path::PathBuf};

use anyhow::Context;
use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::diff::git;

/// Where each of `units` is written, as `{base}/{path in the repository}#L{line}`.
/// With an empty `base`, the base is the commit the repository's `HEAD` is at, on the
/// GitHub repository its `origin` names.
pub(crate) fn source_links<'u>(
    system: &System,
    units: impl IntoIterator<Item = &'u UnitId>,
    base: &str,
) -> anyhow::Result<HashMap<UnitId, String>> {
    let located: Vec<(&UnitId, PathBuf, u32)> = units
        .into_iter()
        .filter_map(|id| {
            let location = system.unit(id)?.location.as_ref()?;
            let path = system.local_path(id, &location.uri)?;
            let line = location.range.as_ref().map_or(1, |r| r.start_line);
            Some((id, path, line))
        })
        .collect();
    let Some((_, first, _)) = located.first() else {
        return Ok(HashMap::new());
    };
    let top = PathBuf::from(git(
        first.parent().unwrap_or(first),
        &["rev-parse", "--show-toplevel"],
    )?);
    let base = if base.is_empty() {
        let remote = git(&top, &["remote", "get-url", "origin"])?;
        let commit = git(&top, &["rev-parse", "HEAD"])?;
        let web = github(&remote).with_context(|| {
            format!("cannot tell where `{remote}` is on the web; pass --links=BASE")
        })?;
        format!("{web}/blob/{commit}")
    } else {
        base.trim_end_matches('/').to_owned()
    };
    // The repository may sit behind a symlink the document's root does not.
    let top = top.canonicalize().unwrap_or(top);
    Ok(located
        .into_iter()
        .filter_map(|(id, path, line)| {
            let path = path.canonicalize().unwrap_or(path);
            let relative = path.strip_prefix(&top).ok()?;
            let relative = relative.to_string_lossy().replace('\\', "/");
            Some((id.clone(), format!("{base}/{relative}#L{line}")))
        })
        .collect())
}

/// `https://github.com/owner/repo` for a GitHub remote, in any of the forms git
/// accepts.
fn github(remote: &str) -> Option<String> {
    let path = remote
        .strip_prefix("git@github.com:")
        .or_else(|| remote.strip_prefix("ssh://git@github.com/"))
        .or_else(|| remote.strip_prefix("https://github.com/"))
        .or_else(|| remote.strip_prefix("http://github.com/"))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    (path.split('/').count() == 2).then(|| format!("https://github.com/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_github_remote_on_the_web() {
        let web = Some("https://github.com/o/r".to_owned());
        assert_eq!(github("git@github.com:o/r.git"), web);
        assert_eq!(github("ssh://git@github.com/o/r.git"), web);
        assert_eq!(github("https://github.com/o/r"), web);
        assert_eq!(github("https://github.com/o/r.git/"), web);
        assert_eq!(github("https://gitlab.com/o/r"), None);
        assert_eq!(github("git@github.com:o"), None);
    }
}
