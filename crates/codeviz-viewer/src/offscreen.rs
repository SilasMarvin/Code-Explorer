//! A scripted session draws into an image, not a window. No OS window is made, so
//! nothing shows on screen, nothing takes the keyboard, and no Space, full-screen app,
//! lock or sleep can blank a snapshot. The primary [`Window`] stays as the viewer's
//! idea of the window: its size, scale and cursor, which the session's hands set
//! (`drive::act`) and every system reads as from a real one. Every camera renders into
//! [`Offscreen`]'s image, kept the window's physical size; egui, whose context rides
//! on a camera, draws its panels there too.

use std::time::Duration;

use bevy::{
    app::ScheduleRunnerPlugin,
    camera::{ImageRenderTarget, RenderTarget},
    input::InputSystems,
    prelude::*,
    render::render_resource::{Extent3d, TextureFormat, TextureUsages},
    window::PrimaryWindow,
};
use bevy_egui::{
    EguiGlobalSettings, EguiPreUpdateSet, PrimaryEguiContext, input::WindowToEguiContextMap,
};

/// Physical pixels to a point, as on the Retina display the sessions were first drawn
/// on: text is laid out and hinted as there, and `tools/drive` scales each PNG back to
/// points.
pub(crate) const SCALE: f32 = 2.0;

pub(crate) struct OffscreenPlugin;

/// The image every camera draws into, and the target that names it.
#[derive(Resource, Clone)]
pub(crate) struct Offscreen(pub(crate) ImageRenderTarget);

impl Plugin for OffscreenPlugin {
    fn build(&self, app: &mut App) {
        // Frames as fast as they draw, with no window's event loop to pace them.
        app.add_plugins(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
            .add_systems(PreStartup, make_image)
            .add_observer(aim_at_image)
            .add_systems(
                PreUpdate,
                (
                    fit_image.after(InputSystems),
                    tie_egui_to_window
                        .after(EguiPreUpdateSet::InitContexts)
                        .before(EguiPreUpdateSet::ProcessInput),
                ),
            );
    }
}

fn make_image(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut egui: ResMut<EguiGlobalSettings>,
) {
    let size = window.physical_size();
    // The format egui's pipeline draws into an image with.
    let mut image = Image::new_target_texture(size.x, size.y, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    commands.insert_resource(Offscreen(ImageRenderTarget {
        handle: images.add(image),
        scale_factor: SCALE,
    }));
    // No OS window to place an IME's box by.
    egui.enable_ime = false;
}

/// Each camera, as it is made, draws into the image instead of the window.
fn aim_at_image(added: On<Add<Camera>>, offscreen: Res<Offscreen>, mut commands: Commands) {
    commands
        .entity(added.entity)
        .insert(RenderTarget::Image(offscreen.0.clone()));
}

/// Keeps the image the window's size, as a `window` step sizes it anew, before the
/// cameras lay out their viewports for the frame. Every camera is told, so none draws
/// a frame at the old size.
fn fit_image(
    offscreen: Res<Offscreen>,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut cameras: Query<&mut Projection, With<Camera>>,
) {
    let size = window.physical_size();
    let Some(image) = images.get(&offscreen.0.handle) else {
        return;
    };
    if image.size() == size {
        return;
    }
    if let Some(mut image) = images.get_mut(&offscreen.0.handle) {
        image.resize(Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
    }
    for mut projection in &mut cameras {
        projection.set_changed();
    }
}

/// egui ties a context to a window by its camera's target, and so leaves one drawing
/// into an image without the window's pointer and keys; this ties the primary context
/// to the primary window, so input reaches it as from a window.
fn tie_egui_to_window(
    mut map: ResMut<WindowToEguiContextMap>,
    context: Single<Entity, With<PrimaryEguiContext>>,
    window: Single<Entity, With<PrimaryWindow>>,
) {
    let (context, window) = (*context, *window);
    if map.context_to_window.get(&context) == Some(&window) {
        return;
    }
    map.window_to_contexts
        .entry(window)
        .or_default()
        .insert(context);
    map.context_to_window.insert(context, window);
}
