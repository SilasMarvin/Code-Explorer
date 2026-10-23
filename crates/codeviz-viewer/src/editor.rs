//! Opening a function in the user's editor: which editor, and the command that opens
//! a file at a line there. Working out the command is pure, so tests and renders
//! never start anything; only [`spawn`] does.

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// The default editor's command, as `$CODEVIZ_EDITOR` would write it.
pub(crate) const DEFAULT: &str = "code -g {file}:{line}";

/// What picks the editor.
#[derive(Debug, Clone, Default)]
pub(crate) struct Env {
    /// `$CODEVIZ_EDITOR`: a command with `{file}`, `{line}` and `{column}` in it, or
    /// an editor's name.
    pub(crate) codeviz_editor: Option<String>,
    pub(crate) visual: Option<String>,
    pub(crate) editor: Option<String>,
    /// VS Code's `code`, found on the `PATH` or in its macOS app.
    pub(crate) code: Option<PathBuf>,
    /// Whether codeviz was started from a terminal, where a terminal editor can run.
    pub(crate) terminal: bool,
}

impl Env {
    pub(crate) fn from_process() -> Self {
        let var = |name| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        let on_path = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join("code"))
                .find(|code| code.is_file())
        });
        let in_app = Some(PathBuf::from(
            "/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code",
        ))
        .filter(|code| cfg!(target_os = "macos") && code.is_file());
        Self {
            codeviz_editor: var("CODEVIZ_EDITOR"),
            visual: var("VISUAL"),
            editor: var("EDITOR"),
            code: on_path.or(in_app),
            terminal: std::io::IsTerminal::is_terminal(&std::io::stdin()),
        }
    }
}

/// A command that opens a file at a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Launch {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    /// The editor's name, for "Opened in …".
    pub(crate) name: String,
    /// Whether it runs in the terminal codeviz was started from, rather than in a
    /// window of its own.
    pub(crate) in_terminal: bool,
}

/// How an editor opens a file at a line, and whether it needs a terminal.
struct Known {
    programs: &'static [&'static str],
    name: &'static str,
    args: &'static str,
    terminal: bool,
}

const KNOWN: &[Known] = &[
    Known {
        programs: &["code", "code-insiders"],
        name: "VS Code",
        args: "-g {file}:{line}",
        terminal: false,
    },
    Known {
        programs: &["codium"],
        name: "VSCodium",
        args: "-g {file}:{line}",
        terminal: false,
    },
    Known {
        programs: &["cursor"],
        name: "Cursor",
        args: "-g {file}:{line}",
        terminal: false,
    },
    Known {
        programs: &["windsurf"],
        name: "Windsurf",
        args: "-g {file}:{line}",
        terminal: false,
    },
    Known {
        programs: &["zed", "zeditor"],
        name: "Zed",
        args: "{file}:{line}",
        terminal: false,
    },
    Known {
        programs: &["subl"],
        name: "Sublime Text",
        args: "{file}:{line}",
        terminal: false,
    },
    Known {
        programs: &["mate"],
        name: "TextMate",
        args: "-l {line} {file}",
        terminal: false,
    },
    Known {
        programs: &[
            "idea",
            "rustrover",
            "pycharm",
            "clion",
            "goland",
            "webstorm",
        ],
        name: "JetBrains",
        args: "--line {line} {file}",
        terminal: false,
    },
    Known {
        programs: &["emacsclient"],
        name: "Emacs",
        args: "-n +{line} {file}",
        terminal: false,
    },
    Known {
        programs: &["emacs"],
        name: "Emacs",
        args: "+{line} {file}",
        terminal: false,
    },
    Known {
        programs: &["gvim", "mvim"],
        name: "Vim",
        args: "+{line} {file}",
        terminal: false,
    },
    Known {
        programs: &["vi", "vim", "nvim", "nano", "micro", "kak", "ne", "joe"],
        name: "",
        args: "+{line} {file}",
        terminal: true,
    },
    Known {
        programs: &["hx", "helix"],
        name: "Helix",
        args: "{file}:{line}",
        terminal: true,
    },
];

/// The command that opens `file` at `line`: `$CODEVIZ_EDITOR`; else `$VISUAL` or
/// `$EDITOR` when it is an editor known to open a window of its own; else VS Code;
/// else `$VISUAL` or `$EDITOR` in the terminal codeviz started from, or on its own
/// without one; else whatever the system opens the file with, at its top.
pub(crate) fn launch(env: &Env, file: &Path, line: u32, column: u32) -> Launch {
    let place = |words: &[String]| fill(words, file, line, column);
    if let Some(command) = &env.codeviz_editor {
        let words = words(command);
        if words.iter().any(|w| w.contains("{file}")) {
            return named(place(&words), false);
        }
        if let Some((launch, _)) = editor(&words, &place, env.terminal) {
            return launch;
        }
    }
    let chosen: Vec<(Launch, bool)> = [&env.visual, &env.editor]
        .into_iter()
        .flatten()
        .filter_map(|command| editor(&words(command), &place, env.terminal))
        .collect();
    let windowed = chosen.iter().find(|(l, known)| *known && !l.in_terminal);
    if let Some((launch, _)) = windowed {
        return launch.clone();
    }
    if let Some(code) = &env.code {
        let mut words = vec![code.display().to_string()];
        words.extend(self::words("-g {file}:{line}"));
        return Launch {
            name: "VS Code".to_owned(),
            ..named(place(&words), false)
        };
    }
    let fits = |l: &Launch| env.terminal || !l.in_terminal;
    if let Some((launch, _)) = chosen.iter().find(|(l, _)| fits(l)) {
        return launch.clone();
    }
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Launch {
        name: "the default app".to_owned(),
        ..named(vec![opener.to_owned(), file.display().to_string()], false)
    }
}

/// `words`, an editor with any options of its own, opening the place, and whether
/// it is an editor we know. One we do not know is given `+LINE FILE`, as most take,
/// and the terminal when there is one.
fn editor(
    words: &[String],
    place: &impl Fn(&[String]) -> Vec<String>,
    terminal: bool,
) -> Option<(Launch, bool)> {
    let program = words.first()?;
    let base = Path::new(program)
        .file_name()
        .map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
    let known = KNOWN.iter().find(|k| k.programs.contains(&base.as_str()));
    let (args, name, in_terminal) = match known {
        Some(known) => (known.args, known.name, known.terminal),
        None => ("+{line} {file}", "", terminal),
    };
    let mut all = words.to_vec();
    all.extend(self::words(args));
    let launch = named(place(&all), in_terminal);
    let launch = if name.is_empty() {
        launch
    } else {
        Launch {
            name: name.to_owned(),
            ..launch
        }
    };
    Some((launch, known.is_some()))
}

/// A launch named after its program.
fn named(mut words: Vec<String>, in_terminal: bool) -> Launch {
    let program = if words.is_empty() {
        String::new()
    } else {
        words.remove(0)
    };
    Launch {
        name: Path::new(&program)
            .file_name()
            .map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned()),
        program,
        args: words,
        in_terminal,
    }
}

/// `words` with `{file}`, `{line}` and `{column}` filled in. Splitting comes first,
/// so a path with spaces stays one argument.
fn fill(words: &[String], file: &Path, line: u32, column: u32) -> Vec<String> {
    words
        .iter()
        .map(|word| {
            word.replace("{file}", &file.display().to_string())
                .replace("{line}", &line.to_string())
                .replace("{column}", &column.to_string())
        })
        .collect()
}

/// A command split into words as a shell would, honoring quotes but not expanding
/// anything.
pub(crate) fn words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    for c in command.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => {
                quote = Some(c);
                word.get_or_insert_default();
            }
            (None, c) if c.is_whitespace() => words.extend(word.take()),
            _ => word.get_or_insert_default().push(c),
        }
    }
    words.extend(word);
    words
}

/// Starts the editor and leaves it running; a terminal editor takes over the terminal
/// codeviz started from.
pub(crate) fn spawn(launch: &Launch) -> std::io::Result<()> {
    let mut command = Command::new(&launch.program);
    command.args(&launch.args);
    if !launch.in_terminal {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    let mut child = command.spawn()?;
    // Reaped when it exits, so it does not linger as a zombie.
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open(env: &Env) -> (String, Vec<String>, String, bool) {
        let launch = launch(env, Path::new("/src/my app/orders.rs"), 42, 5);
        (launch.program, launch.args, launch.name, launch.in_terminal)
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|&w| w.to_owned()).collect()
    }

    #[test]
    fn a_template_fills_in_the_place() {
        let env = Env {
            codeviz_editor: Some("my-edit --at '{file}#L{line}' -c {column}".to_owned()),
            visual: Some("zed".to_owned()),
            ..Env::default()
        };
        let (program, args, name, _) = open(&env);
        assert_eq!(program, "my-edit");
        assert_eq!(
            args,
            strings(&["--at", "/src/my app/orders.rs#L42", "-c", "5"])
        );
        assert_eq!(name, "my-edit");
    }

    #[test]
    fn the_default_reads_as_documented() {
        let env = Env {
            code: Some(PathBuf::from("code")),
            ..Env::default()
        };
        assert_eq!(
            open(&env),
            (
                "code".to_owned(),
                strings(&["-g", "/src/my app/orders.rs:42"]),
                "VS Code".to_owned(),
                false
            )
        );
        assert_eq!(words(DEFAULT)[0], "code");
    }

    #[test]
    fn a_windowed_editor_comes_before_vs_code_and_a_terminal_one_after() {
        let code = Some(PathBuf::from("/usr/local/bin/code"));
        let zed = Env {
            visual: Some("zed --new".to_owned()),
            code: code.clone(),
            ..Env::default()
        };
        assert_eq!(
            open(&zed),
            (
                "zed".to_owned(),
                strings(&["--new", "/src/my app/orders.rs:42"]),
                "Zed".to_owned(),
                false
            )
        );
        let vim = Env {
            editor: Some("nvim".to_owned()),
            code,
            terminal: true,
            ..Env::default()
        };
        assert_eq!(open(&vim).2, "VS Code", "VS Code over a terminal editor");
        let only_vim = Env { code: None, ..vim };
        assert_eq!(
            open(&only_vim),
            (
                "nvim".to_owned(),
                strings(&["+42", "/src/my app/orders.rs"]),
                "nvim".to_owned(),
                true
            )
        );
        let no_terminal = Env {
            terminal: false,
            ..only_vim
        };
        let (program, args, _, in_terminal) = open(&no_terminal);
        assert!(
            ["open", "xdg-open"].contains(&program.as_str()) && !in_terminal,
            "without a terminal, the system opens it: {program} {args:?}"
        );
    }

    #[test]
    fn an_editor_named_in_codeviz_editor_gets_its_own_syntax() {
        let env = Env {
            codeviz_editor: Some("subl -n".to_owned()),
            ..Env::default()
        };
        assert_eq!(open(&env).1, strings(&["-n", "/src/my app/orders.rs:42"]));
    }

    #[test]
    fn splits_words_as_a_shell_does() {
        assert_eq!(
            words(r#"code  -g "a b":{line} 'it''s' x"#),
            strings(&["code", "-g", "a b:{line}", "its", "x"])
        );
        assert_eq!(words(r#"say """#), strings(&["say", ""]));
    }
}
