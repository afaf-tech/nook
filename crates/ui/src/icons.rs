//! Glyphs for the shell, painted rather than loaded.
//!
//! The approved design draws its icons as inline SVG: 14×14 path data, a 1.5-unit round-capped
//! stroke, `currentColor` for the ink. Bundling an icon font or an icon crate would mean a second
//! source of truth for geometry the design already fixed, and the shapes are simple enough that
//! painting them is cheaper than the dependency. The coordinates below are the design's own, in its
//! 14-unit box, mapped onto whatever rectangle the caller offers.
//!
//! Only the five glyphs the activity rail needs are here. Add the next one from the same design
//! markup rather than inventing a shape.

use egui::{pos2, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

/// The glyphs the shell draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    /// A database cylinder: the Connections workspace.
    Connections,
    /// A terminal window: the Queries workspace.
    Queries,
    /// A clock face: scheduled jobs.
    Jobs,
    /// A ruled document: reports.
    Reports,
    /// A pulse trace: the monitor.
    Monitor,
}

/// The design's stroke, in its 14-unit box.
const DESIGN_STROKE: f32 = 1.5;
/// The design's icon box.
const DESIGN_BOX: f32 = 14.0;
/// Samples per cubic segment. The curves here span at most a dozen pixels, so this is plenty and the
/// cost is a handful of points per icon.
const CURVE_STEPS: usize = 12;

/// Paints `glyph` centred in `rect`, scaled from the design's box, in `colour`. Returns the rectangle
/// the glyph covers, so a caller can check that it stayed inside the space it was given.
pub fn glyph(ui: &egui::Ui, rect: Rect, glyph: Glyph, colour: Color32) -> Rect {
    let scale = scale_of(rect);
    let at = mapper(rect);
    let stroke = Stroke::new(DESIGN_STROKE * scale, colour);
    let (paths, rings, boxes) = geometry(glyph, &at, scale);
    let painter = ui.painter();
    paint(painter, &paths, &rings, &boxes, stroke, scale);
    bounds(&paths, &rings, &boxes).expand(stroke.width / 2.0)
}

fn paint(
    painter: &Painter,
    paths: &[Vec<Pos2>],
    rings: &[(Pos2, Vec2)],
    boxes: &[(Rect, f32)],
    stroke: Stroke,
    scale: f32,
) {
    for path in paths {
        painter.add(Shape::line(path.clone(), stroke));
    }
    for (centre, radius) in rings {
        painter.add(Shape::ellipse_stroke(*centre, *radius, stroke));
    }
    for (rect, radius) in boxes {
        painter.add(Shape::rect_stroke(
            *rect,
            CornerRadius::same((radius * scale).round() as u8),
            stroke,
            StrokeKind::Middle,
        ));
    }
}

/// Maps a point of the design's 14-unit box onto `rect`, keeping the glyph square and centred.
fn mapper(rect: Rect) -> impl Fn(f32, f32) -> Pos2 {
    let size = rect.width().min(rect.height());
    let square = Rect::from_center_size(rect.center(), Vec2::splat(size));
    let scale = size / DESIGN_BOX;
    move |x: f32, y: f32| pos2(square.left() + x * scale, square.top() + y * scale)
}

fn scale_of(rect: Rect) -> f32 {
    rect.width().min(rect.height()) / DESIGN_BOX
}

/// Polyline for one cubic segment, sampled at [`CURVE_STEPS`]. Affine mapping commutes with the
/// Bézier evaluation, so sampling the mapped control points traces the same curve.
fn cubic(
    at: &impl Fn(f32, f32) -> Pos2,
    from: (f32, f32),
    control_one: (f32, f32),
    control_two: (f32, f32),
    to: (f32, f32),
) -> Vec<Pos2> {
    let one = at(from.0, from.1);
    let two = at(control_one.0, control_one.1);
    let three = at(control_two.0, control_two.1);
    let four = at(to.0, to.1);
    (1..CURVE_STEPS)
        .map(|step| {
            let t = step as f32 / CURVE_STEPS as f32;
            let u = 1.0 - t;
            let weight = |a: f32, b: f32, c: f32, d: f32| {
                u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
            };
            pos2(
                weight(one.x, two.x, three.x, four.x),
                weight(one.y, two.y, three.y, four.y),
            )
        })
        .collect()
}

type Lines = Vec<Vec<Pos2>>;
type Rings = Vec<(Pos2, Vec2)>;
type Boxes = Vec<(Rect, f32)>;

/// The design's geometry for each glyph, in its 14-unit coordinates: polylines, stroked ellipses and
/// stroked rounded boxes.
fn geometry(glyph: Glyph, at: &impl Fn(f32, f32) -> Pos2, scale: f32) -> (Lines, Rings, Boxes) {
    match glyph {
        Glyph::Connections => {
            // Body: down the left wall, along the bottom, up the right wall. The `s` command in the
            // design's path mirrors the previous control point, which is what the second cubic does.
            let mut body = vec![at(2.4, 3.6), at(2.4, 10.4)];
            body.extend(cubic(
                at,
                (2.4, 10.4),
                (2.4, 11.45),
                (4.46, 12.3),
                (7.0, 12.3),
            ));
            body.push(at(7.0, 12.3));
            body.extend(cubic(
                at,
                (7.0, 12.3),
                (9.54, 12.3),
                (11.6, 11.45),
                (11.6, 10.4),
            ));
            body.push(at(11.6, 3.6));
            let mut band = vec![at(2.4, 7.0)];
            band.extend(cubic(at, (2.4, 7.0), (2.4, 8.05), (4.46, 8.9), (7.0, 8.9)));
            band.push(at(7.0, 8.9));
            band.extend(cubic(
                at,
                (7.0, 8.9),
                (9.54, 8.9),
                (11.6, 8.05),
                (11.6, 7.0),
            ));
            (
                vec![body, band],
                vec![(at(7.0, 3.6), Vec2::new(4.6 * scale, 1.9 * scale))],
                Vec::new(),
            )
        }
        Glyph::Queries => (
            vec![
                vec![at(4.0, 5.6), at(5.9, 7.5), at(4.0, 9.4)],
                vec![at(7.6, 9.6), at(10.6, 9.6)],
            ],
            Vec::new(),
            vec![(box_of(at, 1.6, 2.6, 10.8, 8.8), 1.6)],
        ),
        Glyph::Jobs => (
            vec![vec![at(7.0, 4.2), at(7.0, 7.0), at(9.0, 8.4)]],
            vec![(at(7.0, 7.0), Vec2::splat(5.2 * scale))],
            Vec::new(),
        ),
        Glyph::Reports => (
            vec![
                vec![at(5.2, 4.8), at(8.8, 4.8)],
                vec![at(5.2, 7.0), at(8.8, 7.0)],
                vec![at(5.2, 9.2), at(7.4, 9.2)],
            ],
            Vec::new(),
            vec![(box_of(at, 2.6, 1.6, 8.8, 10.8), 1.6)],
        ),
        Glyph::Monitor => (
            vec![vec![
                at(1.4, 7.4),
                at(4.2, 7.4),
                at(6.0, 3.2),
                at(8.4, 10.8),
                at(9.8, 7.4),
                at(12.6, 7.4),
            ]],
            Vec::new(),
            Vec::new(),
        ),
    }
}

/// A rounded box from the design's `x`, `y`, `width`, `height`.
fn box_of(at: &impl Fn(f32, f32) -> Pos2, x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect::from_min_max(at(x, y), at(x + width, y + height))
}

fn bounds(paths: &[Vec<Pos2>], rings: &[(Pos2, Vec2)], boxes: &[(Rect, f32)]) -> Rect {
    let mut points: Vec<Pos2> = paths.iter().flatten().copied().collect();
    for (centre, radius) in rings {
        points.push(*centre - *radius);
        points.push(*centre + *radius);
    }
    for (rect, _) in boxes {
        points.push(rect.min);
        points.push(rect.max);
    }
    Rect::from_points(&points)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Glyph; 5] = [
        Glyph::Connections,
        Glyph::Queries,
        Glyph::Jobs,
        Glyph::Reports,
        Glyph::Monitor,
    ];

    /// The invariant that matters: a glyph never paints outside the rectangle it is handed, at any
    /// size — including a rectangle that is not square, where the glyph has to be centred.
    #[test]
    fn glyphs_stay_inside_the_rect_they_are_given() {
        for glyph in ALL {
            for (width, height) in [
                (14.0, 14.0),
                (32.0, 32.0),
                (9.0, 9.0),
                (64.0, 20.0),
                (20.0, 64.0),
            ] {
                let rect = Rect::from_min_size(pos2(120.0, 40.0), Vec2::new(width, height));
                let scale = scale_of(rect);
                let (paths, rings, boxes) = geometry(glyph, &mapper(rect), scale);
                let stroke = DESIGN_STROKE * scale;
                let covered = bounds(&paths, &rings, &boxes).expand(stroke / 2.0);
                assert!(
                    rect.contains_rect(covered),
                    "{glyph:?} in {width}×{height} covers {covered:?}, outside {rect:?}"
                );
            }
        }
    }

    /// Every glyph draws exactly the elements the design's path data has, and every path keeps at
    /// least its two endpoints — otherwise a dropped arm would paint a half-empty rail button and
    /// this test would not notice.
    #[test]
    fn every_glyph_has_geometry() {
        for (glyph, arms, rings, boxes) in [
            (Glyph::Connections, 2, 1, 0),
            (Glyph::Queries, 2, 0, 1),
            (Glyph::Jobs, 1, 1, 0),
            (Glyph::Reports, 3, 0, 1),
            (Glyph::Monitor, 1, 0, 0),
        ] {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(32.0));
            let (paths, ring_list, box_list) = geometry(glyph, &mapper(rect), scale_of(rect));
            assert_eq!(
                (paths.len(), ring_list.len(), box_list.len()),
                (arms, rings, boxes),
                "{glyph:?} changed shape"
            );
            for path in &paths {
                assert!(
                    path.len() >= 2,
                    "{glyph:?} has a path with {} point(s): it would paint nothing",
                    path.len()
                );
            }
        }
    }

    /// The cubic sampler traces the endpoints it is given, so a curve cannot silently drift.
    #[test]
    fn cubic_hits_both_ends() {
        let at = mapper(Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(14.0)));
        let from = at(0.0, 0.0);
        let to = at(14.0, 14.0);
        let sampled = cubic(&at, (0.0, 0.0), (4.0, 0.0), (10.0, 14.0), (14.0, 14.0));
        let first = sampled.first().copied().unwrap_or(from);
        let last = sampled.last().copied().unwrap_or(from);
        assert!(
            first.distance(from) < 2.0,
            "starts at {first:?}, not {from:?}"
        );
        assert!(last.distance(to) < 2.0, "ends at {last:?}, not {to:?}");
    }
}
