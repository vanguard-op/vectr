//! Affine transforms for the composition primitives (FEAT-003).
//!
//! A scene element's transform is decomposed — translate, rotate, scale, skew —
//! rather than an ordered list. This module fixes that decomposition to one
//! matrix, `translate · rotate · scale · skew`: a local point is first skewed,
//! then scaled, then rotated, then translated. Rotation and scaling are
//! therefore about the element's origin, so a group's rotation turns its
//! children about the group's own origin, and the whole pipeline is
//! deterministic (NFR-010). A composition's transform is the parent's matrix
//! composed with the child's, which flattens a nested tree into concrete
//! placements.

use serde::{Deserialize, Serialize};

use crate::scene::{ProjectionAxis, Transform};

/// A 2D affine transform, stored as the six meaningful entries of a 3×2
/// matrix:
///
/// ```text
/// | a  c  e |
/// | b  d  f |
/// ```
///
/// A point `(x, y)` maps to `(a·x + c·y + e, b·x + d·y + f)`. It serializes as
/// those six entries, so a render model carries its resolved world transform as
/// data (D-014).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    /// X scale and skew term.
    pub a: f64,
    /// Y skew and scale term.
    pub b: f64,
    /// X skew and scale term.
    pub c: f64,
    /// Y scale and skew term.
    pub d: f64,
    /// X translation.
    pub e: f64,
    /// Y translation.
    pub f: f64,
}

impl Affine {
    /// The identity transform.
    pub const IDENTITY: Self = Self {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    /// A translation.
    pub const fn translate(x: f64, y: f64) -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: x,
            f: y,
        }
    }

    /// A scale.
    pub const fn scale(x: f64, y: f64) -> Self {
        Self {
            a: x,
            b: 0.0,
            c: 0.0,
            d: y,
            e: 0.0,
            f: 0.0,
        }
    }

    /// A rotation, in degrees, about the local origin.
    pub fn rotate(degrees: f64) -> Self {
        let (sin, cos) = degrees.to_radians().sin_cos();
        Self {
            a: cos,
            b: sin,
            c: -sin,
            d: cos,
            e: 0.0,
            f: 0.0,
        }
    }

    /// A skew, in degrees on each axis.
    pub fn skew(x_degrees: f64, y_degrees: f64) -> Self {
        Self {
            a: 1.0,
            b: y_degrees.to_radians().tan(),
            c: x_degrees.to_radians().tan(),
            d: 1.0,
            e: 0.0,
            f: 0.0,
        }
    }

    /// The isometric projection used as a composition helper (FEAT-003).
    ///
    /// The X axis rises to the upper right and the Y axis to the upper left, at
    /// the classic 30° angle.
    pub fn isometric() -> Self {
        let cos = 3.0_f64.sqrt() / 2.0;
        Self {
            a: cos,
            b: 0.5,
            c: -cos,
            d: 0.5,
            e: 0.0,
            f: 0.0,
        }
    }

    /// The projection a `projection` element applies, per its axis (FEAT-003).
    ///
    /// An axis projection flattens the children onto that axis — the x axis
    /// keeps the horizontal coordinate and drops the vertical, the y axis does
    /// the reverse — while `isometric` maps both axes onto the scene's
    /// isometric directions.
    pub fn projection(axis: ProjectionAxis) -> Self {
        match axis {
            ProjectionAxis::X => Self {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 0.0,
                e: 0.0,
                f: 0.0,
            },
            ProjectionAxis::Y => Self {
                a: 0.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                e: 0.0,
                f: 0.0,
            },
            ProjectionAxis::Isometric => Self::isometric(),
        }
    }

    /// Composes two transforms: the result applies `next` first, then `self`.
    ///
    /// This is how a child inherits its parent's transform: the parent composes
    /// over the child.
    pub fn then(self, next: Self) -> Self {
        Self {
            a: self.a * next.a + self.c * next.b,
            b: self.b * next.a + self.d * next.b,
            c: self.a * next.c + self.c * next.d,
            d: self.b * next.c + self.d * next.d,
            e: self.a * next.e + self.c * next.f + self.e,
            f: self.b * next.e + self.d * next.f + self.f,
        }
    }

    /// Applies the transform to a point.
    pub fn apply(self, point: [f64; 2]) -> [f64; 2] {
        [
            self.a * point[0] + self.c * point[1] + self.e,
            self.b * point[0] + self.d * point[1] + self.f,
        ]
    }

    /// Whether every entry is finite.
    pub fn is_finite(self) -> bool {
        self.a.is_finite()
            && self.b.is_finite()
            && self.c.is_finite()
            && self.d.is_finite()
            && self.e.is_finite()
            && self.f.is_finite()
    }

    /// Builds the transform a scene element declares.
    ///
    /// The decomposition is `translate · rotate · scale · skew`: a local point
    /// is first skewed, then scaled, then rotated, then translated. Rotation
    /// and scaling are therefore about the element's own origin, moved into
    /// place by its translation (FEAT-003).
    ///
    /// Returns the name of the first non-finite field, so a malformed transform
    /// is reported rather than silently dropped.
    pub fn from_scene(transform: &Transform) -> Result<Self, &'static str> {
        let fields = [
            ("translateX", transform.translate_x),
            ("translateY", transform.translate_y),
            ("rotate", transform.rotate),
            ("scaleX", transform.scale_x),
            ("scaleY", transform.scale_y),
        ];
        for (name, value) in fields {
            if !value.is_finite() {
                return Err(name);
            }
        }
        for (name, value) in [("skewX", transform.skew_x), ("skewY", transform.skew_y)] {
            if value.is_some_and(|value| !value.is_finite()) {
                return Err(name);
            }
        }

        Ok(
            Self::translate(transform.translate_x, transform.translate_y)
                .then(Self::rotate(transform.rotate))
                .then(Self::scale(transform.scale_x, transform.scale_y))
                .then(Self::skew(
                    transform.skew_x.unwrap_or(0.0),
                    transform.skew_y.unwrap_or(0.0),
                )),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene_transform() -> Transform {
        Transform {
            translate_x: 0.0,
            translate_y: 0.0,
            rotate: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            skew_x: None,
            skew_y: None,
        }
    }

    #[test]
    fn identity_leaves_points_alone() {
        assert_eq!(Affine::IDENTITY.apply([3.0, -4.0]), [3.0, -4.0]);
    }

    #[test]
    fn translation_moves_a_point() {
        assert_eq!(
            Affine::translate(10.0, -5.0).apply([1.0, 1.0]),
            [11.0, -4.0]
        );
    }

    #[test]
    fn rotation_is_about_the_origin_in_degrees() {
        let rotated = Affine::rotate(90.0).apply([1.0, 0.0]);
        assert!((rotated[0] - 0.0).abs() < 1e-12);
        assert!((rotated[1] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn a_group_rotates_its_children_about_its_origin() {
        // A group at (100, 0) rotated 90°: a child at local (10, 0) lands at
        // (100, 10), so the rotation pivots on the group's origin (100, 0).
        let group = Affine::from_scene(&Transform {
            translate_x: 100.0,
            rotate: 90.0,
            ..scene_transform()
        })
        .unwrap();
        let child = Affine::IDENTITY;
        let world = group.then(child).apply([10.0, 0.0]);
        assert!((world[0] - 100.0).abs() < 1e-9, "{world:?}");
        assert!((world[1] - 10.0).abs() < 1e-9, "{world:?}");
    }

    #[test]
    fn composition_applies_the_inner_transform_first() {
        let composed = Affine::translate(10.0, 0.0).then(Affine::scale(2.0, 2.0));
        assert_eq!(composed.apply([1.0, 1.0]), [12.0, 2.0]);
    }

    #[test]
    fn scene_transform_decomposes_into_one_matrix() {
        let transform = Transform {
            translate_x: 5.0,
            translate_y: 7.0,
            rotate: 0.0,
            scale_x: 2.0,
            scale_y: 3.0,
            skew_x: None,
            skew_y: None,
        };
        let affine = Affine::from_scene(&transform).unwrap();
        assert_eq!(affine.apply([1.0, 1.0]), [7.0, 10.0]);
    }

    #[test]
    fn a_local_point_is_skewed_before_it_is_scaled() {
        // skewX 45° maps (0, 1) to (1, 1); scaling by (2, 3) then maps it to
        // (2, 3). The reverse order would give (3, 3).
        let transform = Transform {
            scale_x: 2.0,
            scale_y: 3.0,
            skew_x: Some(45.0),
            ..scene_transform()
        };
        let affine = Affine::from_scene(&transform).unwrap();
        let mapped = affine.apply([0.0, 1.0]);
        assert!((mapped[0] - 2.0).abs() < 1e-9, "{mapped:?}");
        assert!((mapped[1] - 3.0).abs() < 1e-9, "{mapped:?}");
    }

    #[test]
    fn a_non_finite_field_is_named() {
        let transform = Transform {
            scale_x: f64::NAN,
            ..scene_transform()
        };
        assert_eq!(Affine::from_scene(&transform), Err("scaleX"));
    }

    #[test]
    fn the_isometric_projection_lifts_both_axes() {
        let projection = Affine::projection(ProjectionAxis::Isometric);
        let x_axis = projection.apply([1.0, 0.0]);
        let y_axis = projection.apply([0.0, 1.0]);
        assert!(x_axis[0] > 0.0 && x_axis[1] > 0.0);
        assert!(y_axis[0] < 0.0 && y_axis[1] > 0.0);
    }

    #[test]
    fn the_x_projection_keeps_the_horizontal_axis() {
        let projection = Affine::projection(ProjectionAxis::X);
        assert_eq!(projection.apply([3.0, 4.0]), [3.0, 0.0]);
    }

    #[test]
    fn the_y_projection_keeps_the_vertical_axis() {
        let projection = Affine::projection(ProjectionAxis::Y);
        assert_eq!(projection.apply([3.0, 4.0]), [0.0, 4.0]);
    }

    #[test]
    fn projection_axis_names_round_trip() {
        for axis in [
            ProjectionAxis::X,
            ProjectionAxis::Y,
            ProjectionAxis::Isometric,
        ] {
            assert_eq!(ProjectionAxis::from_name(axis.as_str()), Some(axis));
        }
        assert_eq!(ProjectionAxis::from_name("both"), None);
    }
}
