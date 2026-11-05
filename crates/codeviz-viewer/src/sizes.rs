//! Every size a dot is drawn at, decided once: a function's dot by its role alone,
//! kept within bounds on screen however near or far the view, at the depth the camera
//! looks at, and nearer or further back as perspective has it. A module's sphere is
//! [`codeviz_layout::place::module_radius`]'s, the same everywhere; text sizes are
//! [`crate::text`]'s; arrows' widths and heads [`crate::arrows`]'.

/// What a function is to the view, which alone sets how big it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dot {
    /// Lit by nothing, or a caller of the start folded into a count.
    Speck,
    /// What the selection or the journey lights, and callers brought in from outside.
    Lit,
    /// The selection, the journey's start and cursor, the function set aside.
    Focus,
}

impl Dot {
    /// Its radius in the world, before the expanded view's zoom: small beside the
    /// room [`crate::nucleus`] leaves between two functions in the most crowded
    /// module.
    pub(crate) const fn radius(self) -> f32 {
        match self {
            Self::Speck => 0.3,
            Self::Lit => 0.5,
            Self::Focus => 0.7,
        }
    }

    /// The least and most its radius shows on screen, in points: never so small a
    /// lit one cannot be seen or clicked, never so big it covers its neighbours.
    const fn bounds(self) -> (f32, f32) {
        match self {
            Self::Speck => (1.5, 4.0),
            Self::Lit => (2.5, 7.0),
            Self::Focus => (3.5, 9.0),
        }
    }

    /// A radius on screen, in points, kept within its bounds.
    fn clamp(self, on_screen: f32) -> f32 {
        let (least, most) = self.bounds();
        on_screen.clamp(least, most)
    }

    /// How big it shows, in points, showing `own` points for its radius in the world,
    /// `nearer` times as near the camera as what the camera looks at: at that depth
    /// within its bounds, and as perspective has it nearer or further, so depth reads
    /// in a still; never past [`PERSPECTIVE`] of its bounds.
    pub(crate) fn shown(self, own: f32, nearer: f32) -> f32 {
        let (least, most) = self.bounds();
        let nearer = nearer.max(f32::EPSILON);
        (self.clamp(own / nearer) * nearer).clamp(least * PERSPECTIVE.0, most * PERSPECTIVE.1)
    }
}

/// How far past its role's bounds a dot's radius on screen goes, smaller further back
/// and bigger nearer than what the camera looks at.
const PERSPECTIVE: (f32, f32) = (0.6, 1.4);

/// Expanded, how far apart, in points, two functions' dots must show for their names
/// to sit comfortably: one row's height up, or across, the name of the one on the left
/// (`chars` long), its pill, the gap to its dot and the dot beside it.
const ROW_HEIGHT: f32 = 22.0;
const NAME_EXTRA: f32 = 26.0;

/// The share of an expanded module's functions whose names must sit comfortably as it
/// expands: a few long names crowding are named only under the pointer, rather than
/// the module growing for them alone. The wheel takes it on until all of them do.
pub(crate) const COMFORTABLE: f32 = 0.8;

/// How many times further apart `dots` (each where it shows, and how many characters
/// its name has) must show for their names to sit comfortably beside them: each dot
/// clear of every other by a row up, or across by the name of the one on the left,
/// for `share` of them. Below 1 they have room to spare; 0 with fewer than two.
pub(crate) fn comfort(dots: &[(bevy::math::Vec2, usize)], share: f32) -> f32 {
    let across = |chars: usize| crate::text::name_width(chars, crate::text::BODY) + NAME_EXTRA;
    let mut needs: Vec<f32> = dots
        .iter()
        .enumerate()
        .map(|(i, &(at, chars))| {
            dots.iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &(other, other_chars))| {
                    let left = if at.x <= other.x { chars } else { other_chars };
                    let apart = (at - other).abs();
                    let by_row = ROW_HEIGHT / apart.y.max(1e-6);
                    let by_name = across(left) / apart.x.max(1e-6);
                    by_row.min(by_name)
                })
                .fold(0.0, f32::max)
        })
        .collect();
    if needs.len() < 2 {
        return 0.0;
    }
    needs.sort_by(f32::total_cmp);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "an index into a short list"
    )]
    let at = ((needs.len() as f32 * share).ceil() as usize).clamp(1, needs.len()) - 1;
    needs[at]
}

/// Expanded, the least and most share of the free area's smaller side the module's
/// diameter takes: as much as its functions need to sit comfortably
/// ([`comfort`]), between the two.
pub(crate) const EXPANDED_SHARE: (f32, f32) = (0.45, 0.85);

/// The share of the free area's smaller side an expanded module's diameter takes,
/// showing at `share` now with its functions needing `comfort` ([`comfort`]).
pub(crate) fn expanded_share(share: f32, comfort: f32) -> f32 {
    (share * comfort).clamp(EXPANDED_SHARE.0, EXPANDED_SHARE.1)
}

/// Zoomed in as far as the wheel goes, an expanded module's dots show at most this
/// many times as far apart as expanding shows them.
const NEAREST_SPREAD: f32 = 3.0;

/// How much more room than [`comfort`] counts every name needs to be placed: each
/// name nearer its own dot than any other, off the calls lit, with its pill's margin.
const PLACED_ROOM: f32 = 1.5;

/// The most of the free area's smaller side an expanded module's diameter takes as
/// the wheel zooms in, expanded at `expanded` of it, showing at `share` now with every
/// one of its functions needing `every` ([`comfort`] of them all) for its name to sit
/// beside it: as near as all its names fit as placed ([`PLACED_ROOM`]), up to its
/// dots [`NEAREST_SPREAD`] times as far apart, and never less than `least`, as any sphere may fill.
pub(crate) fn nearest_share(expanded: f32, share: f32, every: f32, least: f32) -> f32 {
    (share * every * PLACED_ROOM)
        .min(expanded * NEAREST_SPREAD)
        .max(least)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dot_shows_its_roles_size_at_the_depth_looked_at_and_perspective_about_it() {
        // Far out, every lit dot is at its least where the camera looks, and smaller
        // further back, bigger nearer.
        assert!((Dot::Lit.shown(0.5, 1.0) - 2.5).abs() < 1e-6);
        assert!((Dot::Lit.shown(0.4, 0.8) - 2.0).abs() < 1e-6);
        assert!((Dot::Lit.shown(0.6, 1.2) - 3.0).abs() < 1e-6);
        // Never past its bounds' share.
        assert!((Dot::Lit.shown(0.1, 0.2) - 2.5 * PERSPECTIVE.0).abs() < 1e-6);
        assert!((Dot::Focus.shown(100.0, 3.0) - 9.0 * PERSPECTIVE.1).abs() < 1e-6);
        // In its bounds, as it shows.
        assert!((Dot::Lit.shown(4.0, 0.9) - 4.0).abs() < 1e-5);
    }
}
