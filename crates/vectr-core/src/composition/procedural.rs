//! Seeded procedural generation: the variation the procedures apply to their
//! children's geometry (FEAT-006).
//!
//! Every procedure draws its randomness from one [`Rng`] seeded by the scene's
//! seed and the generating element's stable identifier. The same seed and the
//! same identifier therefore yield byte-identical geometry on every run
//! (NFR-010), while distinct elements and distinct instances of one definition
//! vary because their identifiers differ (FEAT-006). A missing scene seed is
//! recorded as the default `0` and fed in the same way.
//!
//! The helpers here are pure geometry and arithmetic over the concrete shapes
//! the compiler lowers: they carry no diagnostics and reach no external state.

use std::collections::HashMap;

use crate::primitives::{Ellipse, Polygon, Shape};

/// The most features one procedural element may generate; beyond it the
/// generation is refused with a defined size limit rather than allowed to grow
/// without bound (NFR-021).
pub const MAX_PROCEDURAL_FEATURES: usize = 100_000;

/// The seed that applies to a scene that declares none (FEAT-006).
pub const DEFAULT_SEED: i64 = 0;

/// A deterministic seed for one procedural element (FEAT-006).
///
/// The effective seed mixes the scene's seed with the element's stable
/// identifier. After a reusable definition is expanded, an element's identifier
/// carries the placing instance's identifier, so two instances of one definition
/// derive different seeds and each stays reproducible.
pub fn effective_seed(scene_seed: i64, element_id: &str) -> u64 {
    // FNV-1a over the identifier, offset by the scene seed, finished with the
    // SplitMix64 finalizer so nearby seeds decorrelate.
    let mut hash = 0xcbf2_9ce4_8422_2325u64 ^ (scene_seed as u64);
    for byte in element_id.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    mix(hash)
}

/// The SplitMix64 finalizer, an avalanche over a 64-bit value.
fn mix(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// A small, deterministic SplitMix64 generator (FEAT-006).
///
/// The algorithm is fixed, so a seed always yields the same stream across runs
/// and platforms; no value depends on address layout or unordered iteration.
pub struct Rng {
    state: u64,
}

impl Rng {
    /// A generator seeded with the given value.
    pub fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The next 64-bit value in the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        mix(self.state)
    }

    /// The next value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        // The top 53 bits give a double with full mantissa resolution.
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// The next value in `[low, high)`; equal bounds yield `low`.
    pub fn range(&mut self, low: f64, high: f64) -> f64 {
        if high <= low {
            return low;
        }
        low + (high - low) * self.unit()
    }
}

/// A grid of already-placed points, used to keep samples a minimum distance
/// apart without an all-pairs scan (FEAT-006).
struct SpacedSampler {
    spacing: f64,
    grid: HashMap<(i64, i64), Vec<[f64; 2]>>,
}

impl SpacedSampler {
    fn new(spacing: f64) -> Self {
        Self {
            spacing: spacing.max(0.0),
            grid: HashMap::new(),
        }
    }

    /// Whether a candidate sits at least `spacing` from every placed point.
    fn accepts(&self, point: [f64; 2]) -> bool {
        if self.spacing <= 0.0 {
            return true;
        }
        let cell = self.spacing;
        let (cx, cy) = cell_of(point, cell);
        let squared = self.spacing * self.spacing;
        for gx in (cx - 1)..=(cx + 1) {
            for gy in (cy - 1)..=(cy + 1) {
                if let Some(points) = self.grid.get(&(gx, gy)) {
                    for other in points {
                        let dx = other[0] - point[0];
                        let dy = other[1] - point[1];
                        if dx * dx + dy * dy < squared {
                            return false;
                        }
                    }
                }
            }
        }
        true
    }

    fn insert(&mut self, point: [f64; 2]) {
        let cell = if self.spacing > 0.0 {
            self.spacing
        } else {
            1.0
        };
        let key = cell_of(point, cell);
        self.grid.entry(key).or_default().push(point);
    }
}

fn cell_of(point: [f64; 2], cell: f64) -> (i64, i64) {
    let safe = if cell > 0.0 { cell } else { 1.0 };
    (
        (point[0] / safe).floor() as i64,
        (point[1] / safe).floor() as i64,
    )
}

/// The axis-aligned bounds of a set of contours, or `None` when empty.
pub fn bounds(contours: &[Vec<[f64; 2]>]) -> Option<([f64; 2], [f64; 2])> {
    let mut bounds: Option<([f64; 2], [f64; 2])> = None;
    for point in contours.iter().flatten() {
        bounds = Some(match bounds {
            Some((min, max)) => (
                [min[0].min(point[0]), min[1].min(point[1])],
                [max[0].max(point[0]), max[1].max(point[1])],
            ),
            None => (*point, *point),
        });
    }
    bounds
}

/// Whether a point lies inside the region the contours enclose, using the
/// even-odd rule so a hole subtracts from its ring (FEAT-006).
pub fn contains(contours: &[Vec<[f64; 2]>], point: [f64; 2]) -> bool {
    let mut inside = false;
    for contour in contours {
        if contour.len() >= 3 && point_in_polygon(contour, point) {
            inside = !inside;
        }
    }
    inside
}

fn point_in_polygon(contour: &[[f64; 2]], point: [f64; 2]) -> bool {
    let mut inside = false;
    let mut j = contour.len() - 1;
    for i in 0..contour.len() {
        let a = contour[i];
        let b = contour[j];
        let crosses = (a[1] > point[1]) != (b[1] > point[1]);
        if crosses {
            let x = (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0];
            if point[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Samples `count` points inside the region, at least `spacing` apart where the
/// region allows it (FEAT-006).
///
/// A candidate that cannot satisfy the separation within a bounded number of
/// attempts is placed anyway, so the requested count is always honoured and the
/// work stays bounded. The result depends only on the seed, the region and the
/// parameters, never on iteration order.
pub fn sample_points(
    rng: &mut Rng,
    contours: &[Vec<[f64; 2]>],
    bounds: ([f64; 2], [f64; 2]),
    count: usize,
    spacing: f64,
) -> Vec<[f64; 2]> {
    let mut sampler = SpacedSampler::new(spacing);
    let mut points = Vec::with_capacity(count);
    let (min, max) = bounds;
    let (width, height) = (max[0] - min[0], max[1] - min[1]);
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return points;
    }

    const ATTEMPTS: usize = 32;
    for _ in 0..count {
        let mut chosen = None;
        for _ in 0..ATTEMPTS {
            let candidate = [rng.range(min[0], max[0]), rng.range(min[1], max[1])];
            if contains(contours, candidate) && sampler.accepts(candidate) {
                chosen = Some(candidate);
                break;
            }
        }
        let point = chosen.unwrap_or_else(|| {
            // Fall back to any interior point so the count is honoured even
            // when the region cannot hold the requested separation.
            let mut fallback = [min[0], min[1]];
            for _ in 0..ATTEMPTS {
                let candidate = [rng.range(min[0], max[0]), rng.range(min[1], max[1])];
                if contains(contours, candidate) {
                    fallback = candidate;
                    break;
                }
            }
            fallback
        });
        sampler.insert(point);
        points.push(point);
    }
    points
}

/// The number of grid cells triangulation would walk for a region and spacing,
/// or `None` when the region or spacing is degenerate (FEAT-006).
///
/// The count lets a caller refuse an untrusted spacing that would demand
/// unbounded work before any cell is visited (NFR-021).
pub fn triangulation_cells(contours: &[Vec<[f64; 2]>], spacing: f64) -> Option<usize> {
    if !spacing.is_finite() || spacing <= 0.0 {
        return None;
    }
    let (min, max) = bounds(contours)?;
    let width = max[0] - min[0];
    let height = max[1] - min[1];
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let columns = (width / spacing).ceil().max(1.0);
    let rows = (height / spacing).ceil().max(1.0);
    Some(columns.min(u32::MAX as f64) as usize * rows.min(u32::MAX as f64) as usize)
}

/// Decomposes the region into axis-aligned triangles sized by `spacing`
/// (FEAT-006).
///
/// A grid of step `spacing` is laid over the region's bounds and each cell is
/// split into two triangles; a triangle whose centroid falls inside the region
/// is kept. The decomposition is deterministic and bounded by `cap`.
pub fn triangulate(contours: &[Vec<[f64; 2]>], spacing: f64, cap: usize) -> Vec<Shape> {
    let mut triangles = Vec::new();
    if !spacing.is_finite() || spacing <= 0.0 {
        return triangles;
    }
    let Some((min, max)) = bounds(contours) else {
        return triangles;
    };
    let width = max[0] - min[0];
    let height = max[1] - min[1];
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return triangles;
    }

    let columns = (width / spacing).ceil().max(1.0) as usize;
    let rows = (height / spacing).ceil().max(1.0) as usize;
    let step_x = width / columns as f64;
    let step_y = height / rows as f64;

    'outer: for row in 0..rows {
        for column in 0..columns {
            let x0 = min[0] + column as f64 * step_x;
            let y0 = min[1] + row as f64 * step_y;
            let x1 = x0 + step_x;
            let y1 = y0 + step_y;
            for triangle in [
                [[x0, y0], [x1, y0], [x0, y1]],
                [[x1, y0], [x1, y1], [x0, y1]],
            ] {
                let centroid = [
                    (triangle[0][0] + triangle[1][0] + triangle[2][0]) / 3.0,
                    (triangle[0][1] + triangle[1][1] + triangle[2][1]) / 3.0,
                ];
                if contains(contours, centroid) {
                    if triangles.len() >= cap {
                        break 'outer;
                    }
                    triangles.push(Shape::Polygon(Polygon {
                        points: triangle.to_vec(),
                    }));
                }
            }
        }
    }
    triangles
}

/// Generates `count` ornamental features along the contours' outlines, offset
/// by `amount` (FEAT-006).
///
/// Features are spaced by arc length so they follow the outline evenly; each is
/// a small diamond pushed along the outward normal. The per-feature size varies
/// slightly with the seed, so an ornamented edge reads as organic while staying
/// reproducible.
pub fn ornament(
    contours: &[Vec<[f64; 2]>],
    count: usize,
    amount: f64,
    rng: &mut Rng,
) -> Vec<Shape> {
    let mut shapes = Vec::new();
    if count == 0 {
        return shapes;
    }
    let amplitude = if amount.is_finite() {
        amount.abs()
    } else {
        0.0
    };
    let base_size = (amplitude * 0.5).max(0.5);

    let outlines = outlines(contours);
    let total: f64 = outlines.iter().map(|line| line.length).sum();
    if !total.is_finite() || total <= 0.0 {
        return shapes;
    }

    for index in 0..count {
        let target = total * (index as f64 + 0.5) / count as f64;
        let Some((point, normal, centroid)) = point_along(&outlines, target) else {
            continue;
        };
        let outward = orient(normal, point, centroid);
        let size = base_size * (0.75 + 0.5 * rng.unit());
        let center = [
            point[0] + outward[0] * amplitude,
            point[1] + outward[1] * amplitude,
        ];
        shapes.push(Shape::Polygon(Polygon {
            points: vec![
                [center[0], center[1] - size],
                [center[0] + size, center[1]],
                [center[0], center[1] + size],
                [center[0] - size, center[1]],
            ],
        }));
    }
    shapes
}

/// The radius of one stipple dot for a given sampling spacing (FEAT-006).
///
/// Stippling places points, not areas; each point is drawn as a small disc
/// whose radius scales with the sampling spacing so a denser field reads as
/// finer grain, with a defined floor so a dot always survives rasterization.
pub fn stipple_radius(spacing: f64) -> f64 {
    let spacing = if spacing.is_finite() && spacing > 0.0 {
        spacing
    } else {
        0.0
    };
    (spacing * 0.15).max(0.25)
}

/// A disc at one stipple point.
pub fn stipple_dot(point: [f64; 2], radius: f64) -> Shape {
    Shape::Ellipse(Ellipse {
        cx: point[0],
        cy: point[1],
        rx: radius,
        ry: radius,
    })
}

/// One outline polyline and its arc length.
struct Outline {
    points: Vec<[f64; 2]>,
    length: f64,
    centroid: [f64; 2],
}

fn outlines(contours: &[Vec<[f64; 2]>]) -> Vec<Outline> {
    let mut outlines = Vec::new();
    for contour in contours {
        if contour.len() < 2 {
            continue;
        }
        let mut length = 0.0;
        for pair in contour.windows(2) {
            length += distance(pair[0], pair[1]);
        }
        // A closed contour's outline includes the closing edge.
        if contour.len() >= 3 {
            length += distance(contour[contour.len() - 1], contour[0]);
        }
        if length <= 0.0 || !length.is_finite() {
            continue;
        }
        let mut centroid = [0.0, 0.0];
        for point in contour {
            centroid[0] += point[0];
            centroid[1] += point[1];
        }
        centroid[0] /= contour.len() as f64;
        centroid[1] /= contour.len() as f64;
        outlines.push(Outline {
            points: contour.clone(),
            length,
            centroid,
        });
    }
    outlines
}

/// The point, edge normal and contour centroid at an arc-length position.
fn point_along(outlines: &[Outline], target: f64) -> Option<([f64; 2], [f64; 2], [f64; 2])> {
    let mut remaining = target;
    for outline in outlines {
        if remaining > outline.length {
            remaining -= outline.length;
            continue;
        }
        let mut walked = 0.0;
        for index in 0..outline.points.len() {
            let a = outline.points[index];
            let b = outline.points[(index + 1) % outline.points.len()];
            let segment = distance(a, b);
            if segment <= 0.0 {
                continue;
            }
            if walked + segment >= remaining {
                let t = (remaining - walked) / segment;
                let point = [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                let normal = [-((b[1] - a[1]) / segment), (b[0] - a[0]) / segment];
                return Some((point, normal, outline.centroid));
            }
            walked += segment;
        }
    }
    None
}

/// Points a normal away from the contour's centroid, so ornament grows outward.
fn orient(normal: [f64; 2], point: [f64; 2], centroid: [f64; 2]) -> [f64; 2] {
    let outward = [point[0] - centroid[0], point[1] - centroid[1]];
    if normal[0] * outward[0] + normal[1] * outward[1] < 0.0 {
        [-normal[0], -normal[1]]
    } else {
        normal
    }
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_effective_seed_is_stable_and_identifier_sensitive() {
        assert_eq!(effective_seed(7, "a"), effective_seed(7, "a"));
        assert_ne!(effective_seed(7, "a"), effective_seed(7, "b"));
        assert_ne!(effective_seed(7, "a"), effective_seed(8, "a"));
    }

    #[test]
    fn the_generator_is_reproducible_and_in_range() {
        let mut first = Rng::new(42);
        let mut second = Rng::new(42);
        for _ in 0..64 {
            let value = first.unit();
            assert_eq!(value, second.unit());
            assert!((0.0..1.0).contains(&value));
        }
        let mut other = Rng::new(43);
        assert_ne!(Rng::new(42).next_u64(), other.next_u64());
    }

    #[test]
    fn a_point_inside_a_square_is_found() {
        let square = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]];
        assert!(contains(&square, [5.0, 5.0]));
        assert!(!contains(&square, [15.0, 5.0]));
    }

    #[test]
    fn sampling_stays_inside_the_region_and_honours_the_count() {
        let square = vec![vec![[0.0, 0.0], [20.0, 0.0], [20.0, 20.0], [0.0, 20.0]]];
        let bounds = bounds(&square).expect("bounds");
        let mut rng = Rng::new(effective_seed(0, "p"));
        let points = sample_points(&mut rng, &square, bounds, 20, 1.0);
        assert_eq!(points.len(), 20);
        for point in &points {
            assert!(contains(&square, *point), "{point:?}");
        }
    }

    #[test]
    fn triangulation_fills_a_square_and_is_bounded() {
        let square = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]];
        let triangles = triangulate(&square, 5.0, MAX_PROCEDURAL_FEATURES);
        assert!(!triangles.is_empty());
        for triangle in &triangles {
            let Shape::Polygon(polygon) = triangle else {
                panic!("a triangle is a polygon");
            };
            assert_eq!(polygon.points.len(), 3);
        }
    }

    #[test]
    fn ornament_places_the_requested_features() {
        let square = vec![vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]];
        let mut rng = Rng::new(effective_seed(0, "p"));
        let shapes = ornament(&square, 6, 1.0, &mut rng);
        assert_eq!(shapes.len(), 6);
    }
}
