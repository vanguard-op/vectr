//! Repetition and grids: one shape placed `count` times with a spacing
//! (FEAT-003).
//!
//! The scene model carries a repeat as a `count` and a scalar `spacing`, so a
//! repeat lays copies out in a row along the X axis; a grid is that row
//! translated by the caller. The placements are the transforms to apply to
//! each copy, so the compiler keeps one child definition and emits `count`
//! instances.

use crate::scene::{Diagnostic, Diagnostics, Element, Location};

use super::transform::Affine;
use super::{COMPOSITION, COUNT_ZERO};

/// The per-copy placements for a repeat element.
///
/// A count of zero renders nothing and raises a warning rather than failing
/// (FEAT-003). A non-finite spacing is refused, since it would place copies at
/// unreachable coordinates.
pub fn placements(element: &Element, diagnostics: &mut Diagnostics) -> Vec<Affine> {
    let count = element.geometry.count.unwrap_or(0);
    if count == 0 {
        diagnostics.push(
            Diagnostic::warning(
                COUNT_ZERO,
                format!(
                    "repeat `{}` has a count of zero and renders nothing",
                    element.id
                ),
            )
            .with_location(Location::element(element.id.clone())),
        );
        return Vec::new();
    }

    let spacing = element.geometry.spacing.unwrap_or(0.0);
    if !spacing.is_finite() {
        diagnostics.push(
            Diagnostic::error(
                COMPOSITION,
                format!("repeat `{}` must have a finite spacing", element.id),
            )
            .with_location(Location::element_at(
                element.id.clone(),
                "/geometry/spacing",
            )),
        );
        return Vec::new();
    }

    (0..count)
        .map(|index| Affine::translate(index as f64 * spacing, 0.0))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{ElementKind, Geometry, Transform};

    fn repeat(count: Option<u32>, spacing: Option<f64>) -> Element {
        Element {
            id: "r1".to_string(),
            scene_id: "s1".to_string(),
            parent_id: None,
            order: 0,
            name: None,
            kind: ElementKind::Repeat,
            geometry: Geometry {
                count,
                spacing,
                ..Geometry::default()
            },
            transform: Transform {
                translate_x: 0.0,
                translate_y: 0.0,
                rotate: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                skew_x: None,
                skew_y: None,
            },
            fill_token: None,
            stroke_profile_id: None,
            stroke_token: None,
            opacity: 1.0,
            visible: true,
        }
    }

    #[test]
    fn a_repeat_produces_one_placement_per_copy() {
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&repeat(Some(4), Some(25.0)), &mut diagnostics);
        assert_eq!(placements.len(), 4);
        assert_eq!(placements[0], Affine::IDENTITY);
        assert_eq!(placements[3], Affine::translate(75.0, 0.0));
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn a_count_of_zero_renders_nothing_and_warns() {
        let mut diagnostics = Diagnostics::new();
        let placements = placements(&repeat(Some(0), Some(10.0)), &mut diagnostics);
        assert!(placements.is_empty());
        assert!(!diagnostics.has_errors());
        let warning = diagnostics.warnings().next().expect("a warning");
        assert_eq!(warning.code, COUNT_ZERO);
        assert!(warning.message.contains("r1"), "{}", warning.message);
    }

    #[test]
    fn a_missing_count_is_treated_as_zero() {
        let mut diagnostics = Diagnostics::new();
        assert!(placements(&repeat(None, Some(10.0)), &mut diagnostics).is_empty());
        assert_eq!(
            diagnostics.warnings().next().map(|d| d.code.clone()),
            Some(COUNT_ZERO)
        );
    }

    #[test]
    fn a_non_finite_spacing_is_refused() {
        let mut diagnostics = Diagnostics::new();
        assert!(placements(&repeat(Some(3), Some(f64::NAN)), &mut diagnostics).is_empty());
        assert!(diagnostics.has_errors());
    }
}
