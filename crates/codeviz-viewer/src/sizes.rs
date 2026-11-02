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
            Self::Lit => (3.0, 7.0),
            Self::Focus => (4.0, 9.0),
        }
    }

    /// A radius on screen, in points, kept within its bounds.
    pub(crate) fn clamp(self, on_screen: f32) -> f32 {
        let (least, most) = self.bounds();
        on_screen.clamp(least, most)
    }
}
