//! Export a document as a Brother ScanNCut `.fcm` cutting file.
//!
//! The origin is the chosen artboard's top-left corner, which becomes the mat's top-left corner, so
//! art placed on the artboard lands at the same position on the mat. Document points are written as
//! millimetres. Each path or compound path becomes one cut piece; a layer's name picks the tool.

use fcm_core::build::{Contour, Design, PieceSpec, Seg, Tool};
use kurbo::{BezPath, PathEl, Rect};
use vectorcraft_doc::{Document, Node, NodeKind};
pub use vectorcraft_fcm_core as fcm_core;

/// Millimetres per document point.
const MM_PER_PT: f64 = 25.4 / 72.0;
/// Cut area of the 12 x 12 inch mat on the CM900 (as saved by Brother's own software), in mm.
pub const MAT_12X12_MM: (f64, f64) = (296.67, 298.80);

#[derive(Clone, Debug)]
pub struct FcmOptions {
    /// Which artboard is the mat (default the first).
    pub artboard: usize,
    /// Cut area in mm; defaults to the 12 x 12 inch mat.
    pub mat_mm: (f64, f64),
}

impl Default for FcmOptions {
    fn default() -> Self {
        FcmOptions { artboard: 0, mat_mm: MAT_12X12_MM }
    }
}

pub struct FcmOutput {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Layer-name hints: "draw", "emboss" and "foil" pick that tool; anything else cuts.
fn tool_for(layer: &str) -> Tool {
    let l = layer.to_ascii_lowercase();
    if l.contains("draw") || l.contains("pen") {
        Tool::Draw
    } else if l.contains("emboss") {
        Tool::Emboss
    } else if l.contains("foil") {
        Tool::Foil
    } else {
        Tool::Cut
    }
}

struct Walk {
    pieces: Vec<PieceSpec>,
    warnings: Vec<String>,
    origin: (f64, f64),
}

impl Walk {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn mm(&self, p: kurbo::Point) -> (f64, f64) {
        ((p.x - self.origin.0) * MM_PER_PT, (p.y - self.origin.1) * MM_PER_PT)
    }

    fn node(&mut self, n: &Node, tool: Tool) {
        if !n.visible {
            return;
        }
        match &n.kind {
            NodeKind::Layer { template, children, .. } => {
                if *template {
                    return;
                }
                let t = n.name.as_deref().map(tool_for).unwrap_or(tool);
                for c in children {
                    self.node(c, t);
                }
            }
            NodeKind::Group { children, clip } => {
                if *clip {
                    self.warn("clipping masks are not applied: the clipped art is written whole");
                }
                for c in children {
                    self.node(c, tool);
                }
            }
            NodeKind::Path { path, guide, clipping, .. } => {
                if !*guide && !*clipping {
                    self.piece(&[path.to_bezpath()], tool);
                }
            }
            NodeKind::Compound { children, .. } => {
                let paths: Vec<BezPath> = children
                    .iter()
                    .filter_map(|c| match &c.kind {
                        NodeKind::Path { path, .. } => Some(path.to_bezpath()),
                        _ => None,
                    })
                    .collect();
                self.piece(&paths, tool);
            }
            NodeKind::Text(_) => self.warn("type is skipped: outline it (Type > Create Outlines) to cut it"),
            NodeKind::Image(_) => self.warn("placed images are not written to the .fcm"),
            _ => self.warn("some object types (blends, envelopes, symbols, meshes…) are skipped: expand them first"),
        }
    }

    fn piece(&mut self, paths: &[BezPath], tool: Tool) {
        let mut contours = Vec::new();
        for bp in paths {
            let mut cur: Option<Contour> = None;
            let mut last = (0.0, 0.0);
            for el in bp.elements() {
                match *el {
                    PathEl::MoveTo(p) => {
                        if let Some(c) = cur.take() {
                            contours.push(c);
                        }
                        last = self.mm(p);
                        cur = Some(Contour { start: last, segs: vec![], closed: false, tool });
                    }
                    PathEl::LineTo(p) => {
                        let q = self.mm(p);
                        if let Some(c) = cur.as_mut() {
                            c.segs.push(Seg::Line(q));
                        }
                        last = q;
                    }
                    PathEl::QuadTo(c1, p) => {
                        // raise the quadratic to a cubic
                        let (a, b, e) = (last, self.mm(c1), self.mm(p));
                        let k = 2.0 / 3.0;
                        let c1m = (a.0 + k * (b.0 - a.0), a.1 + k * (b.1 - a.1));
                        let c2m = (e.0 + k * (b.0 - e.0), e.1 + k * (b.1 - e.1));
                        if let Some(c) = cur.as_mut() {
                            c.segs.push(Seg::Cubic { c1: c1m, c2: c2m, to: e });
                        }
                        last = e;
                    }
                    PathEl::CurveTo(c1, c2, p) => {
                        let (c1, c2, e) = (self.mm(c1), self.mm(c2), self.mm(p));
                        if let Some(c) = cur.as_mut() {
                            c.segs.push(Seg::Cubic { c1, c2, to: e });
                        }
                        last = e;
                    }
                    PathEl::ClosePath => {
                        if let Some(mut c) = cur.take() {
                            // the machine's closed outlines end on their start point
                            if (last.0 - c.start.0).hypot(last.1 - c.start.1) > 1e-6 {
                                c.segs.push(Seg::Line(c.start));
                            }
                            c.closed = true;
                            contours.push(c);
                        }
                    }
                }
            }
            if let Some(c) = cur.take() {
                contours.push(c);
            }
        }
        contours.retain(|c| !c.segs.is_empty());
        if contours.is_empty() {
            return;
        }
        // The piece origin is the centre of its control-point box, in mat mm.
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for c in &contours {
            let mut pts = vec![c.start];
            for s in &c.segs {
                match *s {
                    Seg::Line(p) => pts.push(p),
                    Seg::Cubic { c1, c2, to } => pts.extend([c1, c2, to]),
                }
            }
            for p in pts {
                x0 = x0.min(p.0);
                x1 = x1.max(p.0);
                y0 = y0.min(p.1);
                y1 = y1.max(p.1);
            }
        }
        self.pieces.push(PieceSpec { contours, position_mm: ((x0 + x1) / 2.0, (y0 + y1) / 2.0) });
    }
}

pub fn export(doc: &Document, opts: &FcmOptions) -> Result<FcmOutput, String> {
    let board = doc.artboards.get(opts.artboard).ok_or_else(|| format!("no artboard {}", opts.artboard + 1))?;
    let r: Rect = board.rect;
    let (w, h) = (r.width() * MM_PER_PT, r.height() * MM_PER_PT);
    let mut walk = Walk { pieces: vec![], warnings: vec![], origin: (r.x0, r.y0) };
    if w > opts.mat_mm.0 + 0.5 || h > opts.mat_mm.1 + 0.5 {
        walk.warn(&format!("the artboard ({w:.1} x {h:.1} mm) is larger than the {:.1} x {:.1} mm cutting area", opts.mat_mm.0, opts.mat_mm.1));
    }
    for l in &doc.layers {
        walk.node(l, Tool::Cut);
    }
    if walk.pieces.is_empty() {
        return Err("nothing to cut: no visible paths on the layers".into());
    }
    // pieces whose box leaves the cutting area
    let off: usize = walk
        .pieces
        .iter()
        .filter(|p| p.position_mm.0 < 0.0 || p.position_mm.1 < 0.0 || p.position_mm.0 > opts.mat_mm.0 || p.position_mm.1 > opts.mat_mm.1)
        .count();
    if off > 0 {
        walk.warn(&format!("{off} piece(s) are centred outside the cutting area"));
    }
    let design = Design { mat_width_mm: opts.mat_mm.0, mat_height_mm: opts.mat_mm.1, pieces: walk.pieces };
    let bytes = fcm_core::write(&design.to_fcm());
    Ok(FcmOutput { bytes, warnings: walk.warnings })
}
