//! Geometry types and numeric kernels.
//!
//! [`Point`], [`Size`], [`Rect`], [`Shape`] and [`Side`] are public. The kernels (`hit`,
//! `monotone`, `polyline`, `stats`) are crate-private pure functions that know nothing of graphs.

pub(crate) mod fp;
pub(crate) mod hit;
pub(crate) mod monotone;
pub(crate) mod polyline;
pub mod shape;
pub(crate) mod stats;

use serde::{Deserialize, Serialize};

/// Tolerance below which two coordinates, lengths or costs count as equal.
pub(crate) const EPS: f64 = 1e-9;

/// Slack for comparisons of values built from several float operations (e.g., sums of gaps).
pub(crate) const SLACK: f64 = 1e-6;

/// A cross product, slope or quadratic coefficient below this counts as zero.
pub(crate) const DEGENERATE: f64 = 1e-12;

/// A squared length below this is zero (about [`EPS`] squared).
pub(crate) const DEGENERATE_SQ: f64 = 1e-18;

/// A bare `(x, y)` point in the caller's space (flow space inside the layered engine).
pub(crate) type Pt = (f64, f64);

/// A node's border shape. Alias of [`shape::NodeShape`].
pub type Shape = shape::NodeShape;

/// One border of a node's bounding box.
///
/// In [`crate::layout::PortInfo`] sides are in output space, so `Top` is the smaller-`y` border
/// whatever the [`crate::Direction`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "lowercase")]
pub enum Side {
    /// The larger-`x` border.
    #[default]
    Right,
    /// The smaller-`x` border.
    Left,
    /// The smaller-`y` border.
    Top,
    /// The larger-`y` border.
    Bottom,
}

impl Side {
    /// Whether this is `Right` or `Left`.
    pub fn horizontal(self) -> bool {
        matches!(self, Side::Right | Side::Left)
    }
    /// Stable small index, for array lookups keyed by side.
    pub fn index(self) -> u8 {
        match self {
            Side::Right => 0,
            Side::Left => 1,
            Side::Top => 2,
            Side::Bottom => 3,
        }
    }
}

/// A point in output (drawing) space, in px.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Point {
    /// Horizontal coordinate, growing to the right.
    pub x: f64,
    /// Vertical coordinate, growing downwards (SVG convention).
    pub y: f64,
}

impl Point {
    /// A point at `(x, y)`.
    pub const fn new(x: f64, y: f64) -> Self {
        Point { x, y }
    }
    /// Both coordinates are finite (neither NaN nor infinite).
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
    }
}

impl From<(f64, f64)> for Point {
    fn from((x, y): (f64, f64)) -> Self {
        Point { x, y }
    }
}

impl From<Point> for (f64, f64) {
    fn from(p: Point) -> Self {
        (p.x, p.y)
    }
}

/// The full extent of an axis-aligned box, in px.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Size {
    /// Extent along `x`.
    pub width: f64,
    /// Extent along `y`.
    pub height: f64,
}

impl Size {
    /// A size of `width` by `height`.
    pub const fn new(width: f64, height: f64) -> Self {
        Size { width, height }
    }
    /// Both extents are finite and strictly positive, as a node's size must be.
    pub fn is_positive(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }
}

impl From<(f64, f64)> for Size {
    fn from((width, height): (f64, f64)) -> Self {
        Size { width, height }
    }
}

/// An axis-aligned rectangle from its top-left `min` to its bottom-right `max`. The default is
/// the empty rectangle at the origin.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Rect {
    /// Smallest `x` and `y`.
    pub min: Point,
    /// Largest `x` and `y`.
    pub max: Point,
}

impl Rect {
    /// The box of `size` centred on `center`.
    pub fn from_center(center: Point, size: Size) -> Self {
        let (hw, hh) = (size.width / 2.0, size.height / 2.0);
        Rect {
            min: Point::new(center.x - hw, center.y - hh),
            max: Point::new(center.x + hw, center.y + hh),
        }
    }
    /// The degenerate rectangle holding just `p`.
    pub fn from_point(p: Point) -> Self {
        Rect { min: p, max: p }
    }
    /// Extent along `x`.
    pub fn width(self) -> f64 {
        self.max.x - self.min.x
    }
    /// Extent along `y`.
    pub fn height(self) -> f64 {
        self.max.y - self.min.y
    }
    /// Midpoint of the rectangle.
    pub fn center(self) -> Point {
        Point::new(
            (self.min.x + self.max.x) / 2.0,
            (self.min.y + self.max.y) / 2.0,
        )
    }
    /// The smallest rectangle containing both `self` and `other`.
    pub fn union(self, other: Rect) -> Rect {
        Rect {
            min: Point::new(self.min.x.min(other.min.x), self.min.y.min(other.min.y)),
            max: Point::new(self.max.x.max(other.max.x), self.max.y.max(other.max.y)),
        }
    }
    /// The rectangle grown to also contain `p`.
    pub fn extend(self, p: Point) -> Rect {
        self.union(Rect::from_point(p))
    }
}

/// Node boxes in flow space (layer axis x, order axis y). Indices `0..n_real` are real nodes, the
/// rest are zero-size routing dummies.
pub(crate) struct Boxes {
    pub cx: Vec<f64>,      // layer-axis centre
    pub cy: Vec<f64>,      // order-axis centre
    pub layer_w: Vec<f64>, // extent along the layer axis
    pub order_w: Vec<f64>, // extent along the order axis
    pub ellipse: Vec<bool>,
    pub n_real: usize,
}

impl Boxes {
    /// The outline of real node `i`, or `None` for a dummy or an index past the table.
    pub fn outline(&self, i: usize) -> Option<shape::Outline> {
        if i >= self.n_real || i >= self.cx.len() {
            return None;
        }
        Some(shape::Outline {
            center: (self.cx[i], self.cy[i]),
            size: (self.layer_w[i], self.order_w[i]),
            shape: if self.ellipse[i] {
                shape::NodeShape::Ellipse
            } else {
                shape::NodeShape::Box
            },
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn rect_from_center_and_union() {
        let r = Rect::from_center(Point::new(10.0, 20.0), Size::new(4.0, 6.0));
        assert_eq!(r.min, Point::new(8.0, 17.0));
        assert_eq!(r.max, Point::new(12.0, 23.0));
        assert_eq!((r.width(), r.height()), (4.0, 6.0));
        assert_eq!(r.center(), Point::new(10.0, 20.0));

        let u = r.extend(Point::new(0.0, 40.0));
        assert_eq!(u.min, Point::new(0.0, 17.0));
        assert_eq!(u.max, Point::new(12.0, 40.0));
    }

    #[test]
    fn size_is_positive_rejects_degenerate_boxes() {
        assert!(Size::new(1.0, 1.0).is_positive());
        assert!(!Size::new(0.0, 1.0).is_positive());
        assert!(!Size::new(-1.0, 1.0).is_positive());
        assert!(!Size::new(f64::NAN, 1.0).is_positive());
        assert!(!Size::new(f64::INFINITY, 1.0).is_positive());
    }
}
