//! Where things sit in the world for each arrangement. The explorer computes this once
//! per change; rendering, picking and labels all read it, so an arrangement is defined
//! in one place.

use bevy::prelude::*;

/// How journeys and the codebase are placed in space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Arrangement {
    /// Journeys stacked top to bottom, facing the camera; the map above them.
    #[default]
    Stacked,
    /// Journeys as panes one behind another, lined up on the cursor's function, over
    /// the map laid on the floor.
    Layers,
    /// The codebase floats as a graph of calls; the journeys are pulled out of it and
    /// lined up in front, each bar tethered to the function it runs.
    Graph,
}

/// One span in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct BarBox {
    pub(crate) center: Vec3,
    pub(crate) size: Vec3,
    /// The point where the span enters.
    pub(crate) start: Vec3,
    /// World offset per tick along the span's time axis.
    pub(crate) axis: Vec3,
    pub(crate) enter: u32,
}

impl BarBox {
    /// The point on the span's time axis at `tick`.
    pub(crate) fn at(&self, tick: u32) -> Vec3 {
        self.start + self.axis * (tick as f32 - self.enter as f32)
    }

    /// The far end along the time axis.
    pub(crate) fn end(&self) -> Vec3 {
        let along = self.axis.normalize_or_zero();
        self.center + along * (along.abs().dot(self.size) / 2.0)
    }

    /// Where its name goes: over the left end of the bar.
    pub(crate) fn label_anchor(&self) -> Vec3 {
        self.start + Vec3::new(0.1, self.size.y / 2.0 + 0.15, 0.0)
    }

    /// The size of a thin slab across the time axis, for marking an exit.
    pub(crate) fn cap(&self) -> Vec3 {
        let along = self.axis.normalize_or_zero().abs();
        let across = Vec3::ONE - along;
        (self.size + Vec3::splat(0.25)) * across + along * 0.16
    }
}

/// Where the codebase map lies: a plane through `origin`, map X along `right` and map
/// Y along `up`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MapFrame {
    pub(crate) origin: Vec3,
    pub(crate) right: Vec3,
    pub(crate) up: Vec3,
    /// Tile size in map units.
    pub(crate) tile: Vec2,
}

impl Default for MapFrame {
    fn default() -> Self {
        Self::wall(Vec3::ZERO, Vec2::ONE)
    }
}

impl MapFrame {
    /// Upright and facing the camera.
    pub(crate) fn wall(origin: Vec3, tile: Vec2) -> Self {
        Self {
            origin,
            right: Vec3::X,
            up: Vec3::Y,
            tile,
        }
    }

    /// Lying on the floor, map Y running away from the camera.
    pub(crate) fn floor(origin: Vec3, tile: Vec2) -> Self {
        Self {
            origin,
            right: Vec3::X,
            up: Vec3::NEG_Z,
            tile,
        }
    }

    pub(crate) fn point(&self, position: [f32; 2]) -> Vec3 {
        self.origin + self.right * position[0] + self.up * position[1]
    }

    /// Out of the map's face, towards the viewer.
    pub(crate) fn normal(&self) -> Vec3 {
        self.right.cross(self.up)
    }

    /// Turns a shape drawn in the XY plane onto the map.
    pub(crate) fn rotation(&self) -> Quat {
        Quat::from_mat3(&Mat3::from_cols(self.right, self.up, self.normal()))
    }

    /// World-space scale for a box `width` × `height` on the map, `thickness` deep.
    pub(crate) fn extent(&self, width: f32, height: f32, thickness: f32) -> Vec3 {
        (self.right * width + self.up * height + self.normal() * thickness).abs()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_maps_run_away_from_the_camera() {
        let floor = MapFrame::floor(Vec3::ZERO, Vec2::ONE);
        assert_eq!(floor.normal(), Vec3::Y);
        assert_eq!(floor.point([2.0, 3.0]), Vec3::new(2.0, 0.0, -3.0));
        assert!((floor.rotation() * Vec3::Y).abs_diff_eq(Vec3::NEG_Z, 1e-6));
        assert_eq!(floor.extent(4.0, 1.0, 0.3), Vec3::new(4.0, 0.3, 1.0));
    }

    #[test]
    fn bars_measure_time_along_their_axis() {
        let bar = BarBox {
            center: Vec3::new(5.0, 0.0, 0.0),
            size: Vec3::new(10.0, 0.8, 0.8),
            start: Vec3::ZERO,
            axis: Vec3::X * 2.5,
            enter: 2,
        };
        assert_eq!(bar.at(4), Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(bar.end(), Vec3::new(10.0, 0.0, 0.0));
    }
}
