//! What reaches outside the viewer: opening the focus in the editor (E), copying to
//! the clipboard, saving a picture of the view (P); each says what it did in a brief
//! toast. A screenshot run never starts an editor or touches the clipboard: it says
//! what it would have done, and saves pictures beside its screenshot.

use std::path::{Path, PathBuf};

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
};
use bevy_egui::{EguiClipboard, egui, input::egui_wants_any_keyboard_input};
use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::{diff_view::DiffView, editor, explorer::Explorer, peek::Peek};

pub(crate) struct ActionsPlugin;

impl Plugin for ActionsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Toast>()
            .init_resource::<Requests>()
            .init_resource::<EditRoots>()
            .insert_resource(EditorEnv(editor::Env::from_process()))
            .add_systems(
                Update,
                (keys.run_if(not(egui_wants_any_keyboard_input)), perform).chain(),
            );
    }
}

/// Whether actions reach outside the viewer, or only say what they would do: a
/// screenshot run saves its pictures in `dir`, beside the screenshot.
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) enum Effects {
    Live,
    Render { dir: PathBuf },
}

/// How the editor is picked, read from the environment at start.
#[derive(Resource, Debug)]
struct EditorEnv(editor::Env);

/// Directories whose files are edited elsewhere: a diff's scratch checkout of the
/// newer revision, edited in the working tree.
#[derive(Resource, Debug, Default)]
pub(crate) struct EditRoots(pub(crate) Vec<(PathBuf, PathBuf)>);

impl EditRoots {
    fn map(&self, file: &Path) -> PathBuf {
        self.0
            .iter()
            .find_map(|(from, to)| Some(to.join(file.strip_prefix(from).ok()?)))
            .unwrap_or_else(|| file.to_path_buf())
    }
}

/// Something to do outside the viewer, asked for by a key or a click.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    /// Open the focused function in the editor.
    EditFocus,
    /// Open a file at a line in the editor.
    Edit(Place),
    /// Put text on the clipboard; `what` names it in the toast.
    Copy { text: String, what: String },
    /// Save a picture of the window.
    Picture,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct Requests(pub(crate) Vec<Request>);

impl Requests {
    pub(crate) fn push(&mut self, request: Request) {
        self.0.push(request);
    }
}

/// Where something is written: its file on disk, and line and column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    pub(crate) file: PathBuf,
    pub(crate) line: u32,
    pub(crate) column: u32,
}

impl Place {
    /// Where `unit` is defined, when it has a location on disk.
    pub(crate) fn of(system: &System, unit: &UnitId) -> Option<Self> {
        let location = system.unit(unit)?.location.as_ref()?;
        Some(Self {
            file: system.local_path(unit, &location.uri)?,
            line: location.range.map_or(1, |r| r.start_line),
            column: location.range.map_or(1, |r| r.start_col),
        })
    }
}

/// A line of feedback over the graph, for a few seconds.
#[derive(Resource, Debug, Default)]
pub(crate) struct Toast {
    text: String,
    failed: bool,
    /// When it went up, in seconds of app time; `None` once it is down.
    since: Option<f32>,
}

/// How long a toast stays, and how long it takes to fade.
const TOAST_SECONDS: f32 = 3.5;
const TOAST_FADE: f32 = 0.4;

impl Toast {
    pub(crate) fn show(&mut self, text: impl Into<String>, now: f32) {
        *self = Self {
            text: text.into(),
            failed: false,
            since: Some(now),
        };
    }

    pub(crate) fn fail(&mut self, text: impl Into<String>, now: f32) {
        *self = Self {
            text: text.into(),
            failed: true,
            since: Some(now),
        };
    }

    fn clear(&mut self) {
        self.since = None;
    }

    /// Its text, and how opaque it is at `now`, while it is up.
    fn showing(&self, now: f32) -> Option<(&str, bool, f32)> {
        let age = now - self.since?;
        let opacity = ((TOAST_SECONDS - age) / TOAST_FADE).clamp(0.0, 1.0);
        (age >= 0.0 && opacity > 0.0).then_some((self.text.as_str(), self.failed, opacity))
    }
}

/// Draws the toast at the top of the graph, in the middle.
pub(crate) fn draw_toast(ctx: &egui::Context, area: egui::Rect, toast: &Toast, now: f32) {
    let Some((text, failed, opacity)) = toast.showing(now) else {
        return;
    };
    let color = if failed {
        egui::Color32::from_rgb(240, 150, 140)
    } else {
        egui::Color32::from_rgb(225, 229, 238)
    };
    egui::Area::new("toast".into())
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(area.center().x, area.min.y + 14.0))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            ui.multiply_opacity(opacity);
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(24, 27, 36, 240))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(70)))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_max_width(area.width() * 0.7);
                    ui.label(egui::RichText::new(text).color(color).size(13.5));
                });
        });
    ctx.request_repaint();
}

/// E opens the focus in the editor, P saves a picture of the view, B shows or hides
/// the focus's source over the graph.
fn keys(keys: Res<ButtonInput<KeyCode>>, mut requests: ResMut<Requests>, mut peek: ResMut<Peek>) {
    if keys.just_pressed(KeyCode::KeyE) {
        requests.push(Request::EditFocus);
    }
    if keys.just_pressed(KeyCode::KeyP) {
        requests.push(Request::Picture);
    }
    if keys.just_pressed(KeyCode::KeyB) {
        peek.0 = !peek.0;
    }
}

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn perform(
    mut commands: Commands,
    mut requests: ResMut<Requests>,
    mut toast: ResMut<Toast>,
    mut clipboard: ResMut<EguiClipboard>,
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    effects: Res<Effects>,
    env: Res<EditorEnv>,
    roots: Res<EditRoots>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    for request in std::mem::take(&mut requests.0) {
        match request {
            Request::EditFocus => match focus_place(&explorer, diff.as_deref()) {
                Ok(place) => edit(&place, &env.0, &roots, &effects, &mut toast, now),
                Err(why) => toast.fail(why, now),
            },
            Request::Edit(place) => edit(&place, &env.0, &roots, &effects, &mut toast, now),
            Request::Copy { text, what } => {
                if *effects == Effects::Live {
                    clipboard.set_text(&text);
                } else {
                    info!("would copy: {text}");
                }
                toast.show(format!("Copied {what}"), now);
            }
            Request::Picture => {
                let dir = match &*effects {
                    Effects::Live => pictures_dir(),
                    Effects::Render { dir } => dir.clone(),
                };
                let name = explorer
                    .graph_focus()
                    .and_then(|tile| explorer.tile_unit(tile))
                    .map(|unit| explorer.unit_name(unit));
                let path = dir.join(picture_name(name.as_deref(), unix_seconds()));
                // The toast would be in the picture.
                toast.clear();
                let live = *effects == Effects::Live;
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(path.clone()))
                    .observe(
                        move |captured: On<ScreenshotCaptured>,
                              mut toast: ResMut<Toast>,
                              mut clipboard: ResMut<EguiClipboard>,
                              time: Res<Time>| {
                            let copied = live && copy_image(&captured.image, &mut clipboard);
                            let also = if copied { ", and copied" } else { "" };
                            toast.show(
                                format!("Saved to {}{also}", home_relative(&path)),
                                time.elapsed_secs(),
                            );
                        },
                    );
            }
        }
    }
}

/// Where the focused function is written; or why it cannot be opened.
fn focus_place(explorer: &Explorer, diff: Option<&DiffView>) -> Result<Place, String> {
    let unit = explorer
        .graph_focus()
        .and_then(|tile| explorer.tile_unit(tile))
        .or_else(|| explorer.focus())
        .ok_or("Nothing is focused: click a function first")?;
    let name = explorer.unit_name(unit);
    if explorer.system().unit(unit).is_none() && diff.is_some() {
        return Err(format!("{name} was removed: B shows its old source"));
    }
    Place::of(explorer.system(), unit).ok_or_else(|| format!("{name} has no source on disk"))
}

fn edit(
    place: &Place,
    env: &editor::Env,
    roots: &EditRoots,
    effects: &Effects,
    toast: &mut Toast,
    now: f32,
) {
    let file = roots.map(&place.file);
    if !file.is_file() {
        toast.fail(
            format!("Cannot find {}: pass --root DIR", home_relative(&file)),
            now,
        );
        return;
    }
    let launch = editor::launch(env, &file, place.line, place.column);
    let opened = format!(
        "Opened {}:{} in {}",
        file.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        place.line,
        launch.name
    );
    if *effects != Effects::Live {
        info!("would run: {} {:?}", launch.program, launch.args);
        toast.show(opened, now);
        return;
    }
    match editor::spawn(&launch) {
        Ok(()) if launch.in_terminal => {
            toast.show(format!("{opened}, in the terminal codeviz runs in"), now);
        }
        Ok(()) => toast.show(opened, now),
        Err(error) => toast.fail(
            format!(
                "Cannot run {}: {error}. Set $CODEVIZ_EDITOR, e.g. \"{}\"",
                launch.program,
                editor::DEFAULT
            ),
            now,
        ),
    }
}

/// Puts a screenshot on the clipboard as an image. Returns whether it did.
fn copy_image(image: &Image, clipboard: &mut EguiClipboard) -> bool {
    let Ok(picture) = image.clone().try_into_dynamic() else {
        return false;
    };
    // Its alpha holds brightness, not transparency, as when saving it.
    let rgb = picture.to_rgb8();
    let size = [rgb.width() as usize, rgb.height() as usize];
    clipboard.set_image(&egui::ColorImage::from_rgb(size, rgb.as_raw()));
    true
}

/// Where pictures go: `$XDG_PICTURES_DIR`, else on a Mac the desktop, as its own
/// screenshots do, else `~/Pictures`, else home.
fn pictures_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let xdg = std::env::var_os("XDG_PICTURES_DIR").map(PathBuf::from);
    choose_pictures_dir(xdg, home, cfg!(target_os = "macos"), Path::is_dir)
}

fn choose_pictures_dir(
    xdg: Option<PathBuf>,
    home: Option<PathBuf>,
    mac: bool,
    exists: impl Fn(&Path) -> bool,
) -> PathBuf {
    let home = home.unwrap_or_else(|| PathBuf::from("."));
    let preferred = if mac { "Desktop" } else { "Pictures" };
    xdg.into_iter()
        .chain([home.join(preferred)])
        .find(|dir| exists(dir))
        .unwrap_or(home)
}

/// `codeviz-orders.place-20261023-142301.png`: what the view focuses on, and when,
/// in UTC.
fn picture_name(focus: Option<&str>, unix_seconds: u64) -> String {
    let focus: String = focus
        .unwrap_or("view")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let days = i64::try_from(unix_seconds / 86_400).unwrap_or_default();
    let (year, month, day) = civil(days);
    let seconds = unix_seconds % 86_400;
    format!(
        "codeviz-{focus}-{year:04}{month:02}{day:02}-{:02}{:02}{:02}.png",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A path with the home directory written `~`.
pub(crate) fn home_relative(path: &Path) -> String {
    std::env::var_os("HOME")
        .and_then(|home| path.strip_prefix(home).ok())
        .map_or_else(
            || path.display().to_string(),
            |rest| format!("~/{}", rest.display()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_picture_by_focus_and_time() {
        assert_eq!(
            picture_name(Some("POST /users/{id}"), 1_792_765_381),
            "codeviz-POST__users__id_-20261023-142301.png"
        );
        assert_eq!(picture_name(None, 0), "codeviz-view-19700101-000000.png");
        assert_eq!(civil(-1), (1969, 12, 31));
        assert_eq!(civil(11_016), (2000, 2, 29));
    }

    #[test]
    fn saves_pictures_where_the_system_keeps_them() {
        let home = Some(PathBuf::from("/home/me"));
        let all = |_: &Path| true;
        assert_eq!(
            choose_pictures_dir(None, home.clone(), true, all),
            Path::new("/home/me/Desktop")
        );
        assert_eq!(
            choose_pictures_dir(Some("/pics".into()), home.clone(), false, all),
            Path::new("/pics")
        );
        assert_eq!(
            choose_pictures_dir(None, home, false, |p: &Path| p.ends_with("me")),
            Path::new("/home/me"),
            "home when there is no Pictures"
        );
    }

    #[test]
    fn edits_a_checkout_in_the_working_tree() {
        let roots = EditRoots(vec![(
            PathBuf::from("/tmp/codeviz-1-abc/crates/a"),
            PathBuf::from("/work/repo/crates/a"),
        )]);
        assert_eq!(
            roots.map(Path::new("/tmp/codeviz-1-abc/crates/a/src/lib.rs")),
            Path::new("/work/repo/crates/a/src/lib.rs")
        );
        assert_eq!(
            roots.map(Path::new("/elsewhere.rs")),
            Path::new("/elsewhere.rs")
        );
    }

    #[test]
    fn a_toast_fades_after_a_while() {
        let mut toast = Toast::default();
        assert!(toast.showing(0.0).is_none());
        toast.show("Copied", 10.0);
        assert_eq!(toast.showing(11.0), Some(("Copied", false, 1.0)));
        assert!(
            toast
                .showing(10.0 + TOAST_SECONDS - TOAST_FADE / 2.0)
                .is_some_and(|t| t.2 < 1.0)
        );
        assert!(toast.showing(10.0 + TOAST_SECONDS).is_none());
    }
}
