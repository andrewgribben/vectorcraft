//! `document.cutLines`: cutting outlines from traced shapes.
//!
//! The user traces a scan of printed stickers (Object > Image Trace > Make and Expand), selects the result
//! and runs this. The shapes are united (parts of one sticker that touch become one), holes are filled so
//! only the outside edge is cut, each piece grows by a margin, and the outlines go on their own layer (named
//! "Cut lines", which the FCM export reads as cut) as thin red lines over the scan.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData, SubPath};
use vectorcraft_pathops as po;
use vectorcraft_pathops::BoolOp;

use super::super::edit::selected_roots;
use super::super::pathops::{node_path, shape_node};
use super::super::*;
use super::fcm::num;
use kurbo::Shape;

const MM_PER_PT: f64 = 25.4 / 72.0;
const LAYER: &str = "Cut lines";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "document.cutLines",
        "Make Cut Lines",
        [],
        None,
        "{marginMm?: 1 (0 to 50: how far each outline stands off the art), fillHoles?: true (cut only the outside edge; false keeps holes as holes), minAreaMm2?: 4 (pieces smaller than this are left out: specks), smoothMm?: 0.15 (0 to 5: each outline is simplified to within this before it is grown: a trace has thousands of tiny segments, and the cutter is no more accurate than about 0.25 mm), layer?: \"Cut lines\" (the layer the outlines go on; created if missing)} → {layer, ids, pieces, skipped, overlaps, marginMm}; from the selected shapes (usually an expanded Image Trace of a scan): unites shapes that touch, fills holes, grows each piece by the margin with round corners and draws the outlines as thin red paths on the layer. A layer name that contains draw, pen, emboss or foil picks that tool in the FCM export; any other name cuts. `overlaps` counts outlines that run into each other (stickers closer than two margins)",
        has_selection,
        cut_lines
    )]
}

/// The paths under `n`, in paint order: visible, not guides or clipping paths.
fn collect(n: &Node, out: &mut Vec<(PathData, FillRule)>) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } => {
            if let Some(x) = node_path(n) {
                out.push(x);
            }
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                collect(c, out);
            }
        }
        _ => {}
    }
}

/// The separate pieces of a united path: each outside edge (with its holes when `fill_holes` is off).
fn pieces(united: &PathData, fill_holes: bool) -> Vec<PathData> {
    let subs: Vec<&SubPath> = united.subpaths.iter().filter(|s| s.closed && s.anchors.len() >= 3).collect();
    let beziers: Vec<kurbo::BezPath> = subs.iter().map(|s| PathData::single((*s).clone()).to_bezpath()).collect();
    let start = |i: usize| subs.get(i).and_then(|s| s.anchors.first()).map(|a| a.p);
    // How many other subpaths enclose each one: even depth is an outside edge, odd is a hole.
    let depth: Vec<usize> = (0..subs.len())
        .map(|i| match start(i) {
            Some(p) => beziers.iter().enumerate().filter(|(j, b)| *j != i && b.winding(p) != 0).count(),
            None => 0,
        })
        .collect();
    let mut out = vec![];
    for (i, sp) in subs.iter().enumerate() {
        let d = depth.get(i).copied().unwrap_or(0);
        if d != 0 && fill_holes {
            continue;
        }
        if d % 2 == 1 {
            continue; // a hole: taken with its outside edge below
        }
        let mut parts = vec![(*sp).clone()];
        if !fill_holes {
            for (h, hole) in subs.iter().enumerate() {
                let hd = depth.get(h).copied().unwrap_or(0);
                let inside = start(h).and_then(|p| beziers.get(i).map(|b| b.winding(p) != 0)).unwrap_or(false);
                if hd == d + 1 && inside {
                    parts.push((*hole).clone());
                }
            }
        }
        out.push(PathData::new(parts));
    }
    out
}

fn cut_lines(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "document.cutLines";
    let margin_mm = num(c, p, "marginMm", 1.0)?;
    let min_area_mm2 = num(c, p, "minAreaMm2", 4.0)?;
    if !(0.0..=50.0).contains(&margin_mm) {
        return Err(bad(c, "marginMm must be between 0 and 50"));
    }
    if !(0.0..=1.0e6).contains(&min_area_mm2) {
        return Err(bad(c, "minAreaMm2 must be a positive number"));
    }
    let smooth_mm = num(c, p, "smoothMm", 0.15)?;
    if !(0.0..=5.0).contains(&smooth_mm) {
        return Err(bad(c, "smoothMm must be between 0 and 5"));
    }
    let fill_holes = p.get("fillHoles").and_then(Value::as_bool).unwrap_or(true);
    let name = str_param(p, "layer").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(LAYER).to_string();
    if name.chars().count() > 64 {
        return Err(bad(c, "layer: a name of at most 64 characters"));
    }

    let mut shapes = vec![];
    {
        let d = &s.doc()?.doc;
        for id in selected_roots(s)? {
            if let Some(n) = d.node(id) {
                collect(n, &mut shapes);
            }
        }
    }
    if shapes.is_empty() {
        return Err(bad(c, "select the traced shapes (paths) first"));
    }
    let refs: Vec<(&PathData, FillRule)> = shapes.iter().map(|(path, rule)| (path, *rule)).collect();
    let united = po::unite_all(&refs);

    let delta = margin_mm / MM_PER_PT;
    let (mut outlines, mut skipped) = (vec![], 0usize);
    for piece in pieces(&united, fill_holes) {
        if po::area(&piece, FillRule::NonZero) * MM_PER_PT * MM_PER_PT < min_area_mm2 {
            skipped += 1;
            continue;
        }
        // Simplify first: offsetting a trace's thousands of tiny segments takes minutes, and the result is no
        // more accurate than the cutter.
        let piece = if smooth_mm > 0.0 { po::simplify(&piece, smooth_mm / MM_PER_PT) } else { piece };
        let grown = if delta > 0.0 { po::offset_path(&piece, delta, po::Join::Round, 4.0) } else { piece };
        if !grown.is_empty() {
            outlines.push(grown);
        }
    }
    if outlines.is_empty() {
        return Err(bad(c, format!("nothing left to cut: {skipped} piece(s) were smaller than minAreaMm2 ({min_area_mm2})")));
    }
    let overlaps = count_overlaps(&outlines);

    // The layer: one that is already there, else a new one on top.
    let existing = s.doc()?.doc.layers.iter().find(|l| l.name.as_deref() == Some(name.as_str())).map(|l| l.id);
    let layer = match existing {
        Some(id) => id,
        None => {
            let r = s.execute("layer.new", &json!({"name": name, "top": true}))?;
            let id = r["id"].as_u64().ok_or_else(|| bad(c, "the layer was not created"))?;
            vectorcraft_doc::NodeId(id)
        }
    };
    if !s.doc()?.doc.is_editable(layer) {
        return Err(bad(c, format!("the layer \"{name}\" is locked or hidden")));
    }
    let look = Appearance::basic(Paint::None, Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 0.5);
    let ids = s.edit("Make Cut Lines", |d, sel| {
        let mut ids = vec![];
        for path in outlines {
            let mut n = shape_node(d, path, None);
            n.appearance = look.clone().into();
            ids.push(d.insert(Some(layer), usize::MAX, n)?);
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    Ok(
        json!({"layer": layer.0, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "pieces": ids.len(), "skipped": skipped, "overlaps": overlaps, "marginMm": margin_mm}),
    )
}

/// How many pairs of outlines run into each other (more than a hair of shared area).
fn count_overlaps(outlines: &[PathData]) -> usize {
    let boxes: Vec<Option<kurbo::Rect>> = outlines.iter().map(PathData::bounds).collect();
    let mut n = 0;
    for (i, a) in outlines.iter().enumerate() {
        for (j, b) in outlines.iter().enumerate().skip(i + 1) {
            let near = match (boxes.get(i).copied().flatten(), boxes.get(j).copied().flatten()) {
                (Some(x), Some(y)) => x.x0 < y.x1 && y.x0 < x.x1 && x.y0 < y.y1 && y.y0 < x.y1,
                _ => false,
            };
            if !near {
                continue;
            }
            let both = po::boolean(a, FillRule::NonZero, b, FillRule::NonZero, BoolOp::Intersect);
            if po::area(&both, FillRule::NonZero) * MM_PER_PT * MM_PER_PT > 0.05 {
                n += 1;
            }
        }
    }
    n
}
