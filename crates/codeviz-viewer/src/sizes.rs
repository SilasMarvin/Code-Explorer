//! Every size a sphere or a dot is drawn at, decided once: a module's sphere by how
//! many functions it holds, the same everywhere; a function's dot by its role alone,
//! kept within bounds on screen however near or far the view. Text sizes are
//! [`crate::text`]'s; arrows' widths and heads [`crate::arrows`]'.

pub(crate) use codeviz_layout::atlas::module_radius;

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
    pub(crate) fn clamp(self, on_screen: f32) -> f32 {
        let (least, most) = self.bounds();
        on_screen.clamp(least, most)
    }
}

/// Expanded, how far apart, in points, two functions' dots must show for their names
/// to sit comfortably: one row's height up, or across, the name of the one on the left
/// (`chars` long), its pill, the gap to its dot and the dot beside it.
const ROW_HEIGHT: f32 = 22.0;
const NAME_EXTRA: f32 = 26.0;
/// A character's width as a share of the name's font size.
const CHAR_WIDTH: f32 = 0.55;

/// The share of an expanded module's functions whose names must sit comfortably: a
/// few long names crowding are named only under the pointer, rather than the module
/// growing for them alone.
const COMFORTABLE: f32 = 0.8;

/// How many times further apart `dots` (each where it shows, and how many characters
/// its name has) must show for their names to sit comfortably beside them: each dot
/// clear of every other by a row up, or across by the name of the one on the left,
/// for [`COMFORTABLE`] of them. Below 1 they have room to spare; 0 with fewer than
/// two.
pub(crate) fn comfort(dots: &[(bevy::math::Vec2, usize)]) -> f32 {
    let across = |chars: usize| {
        #[expect(clippy::cast_precision_loss, reason = "a name's length")]
        let chars = chars as f32;
        chars * crate::text::BODY * CHAR_WIDTH + NAME_EXTRA
    };
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
    let at = ((needs.len() as f32 * COMFORTABLE).ceil() as usize).clamp(1, needs.len()) - 1;
    needs[at]
}

/// Expanded, the least and most share of the free area's smaller side the module's
/// diameter takes: as much as its functions need to sit comfortably
/// ([`comfort`]), between the two.
pub(crate) const EXPANDED_SHARE: (f32, f32) = (0.35, 0.85);

/// A module of this many functions or fewer is small: expanded in a journey, it
/// comes no nearer than keeps the journey's modules it calls or is called from on
/// screen beside it.
pub(crate) const SMALL_MODULE: usize = 6;

/// The share of the free area's smaller side an expanded module's diameter takes,
/// showing at `share` now with its functions needing `comfort` ([`comfort`]).
pub(crate) fn expanded_share(share: f32, comfort: f32) -> f32 {
    (share * comfort).clamp(EXPANDED_SHARE.0, EXPANDED_SHARE.1)
}
