//! Export a document as a Brother ScanNCut `.fcm` cutting file.
//!
//! The origin is the chosen artboard's top-left corner, which becomes the mat's top-left corner, so
//! art placed on the artboard lands at the same position on the mat. Document points are written as
//! millimetres. Each path or compound path becomes one cut piece; a layer's name picks the tool.

use fcm_core::build::{Contour, Design, PieceSpec, Seg, Tool};
use kurbo::{BezPath, PathEl, Rect};
use vectorcraft_doc::{Document, Node, NodeKind};
pub mod scan;

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

/// One layer and the tool its name picks.
#[derive(Clone, Debug, PartialEq)]
pub struct LayerInfo {
    pub name: String,
    pub tool: &'static str,
    /// Pieces written from this layer (nested layers and groups included).
    pub pieces: usize,
}

/// What an export would write, without writing it.
#[derive(Clone, Debug)]
pub struct FcmSummary {
    pub artboard_mm: (f64, f64),
    pub cutting_area_mm: (f64, f64),
    pub pieces: usize,
    /// Pieces per tool, in the order cut, draw, emboss, foil.
    pub by_tool: [(&'static str, usize); 4],
    pub layers: Vec<LayerInfo>,
    /// Paths that are not closed, on any tool.
    pub open_paths: usize,
    pub off_mat: usize,
    pub warnings: Vec<String>,
}

/// The tool's name as shown to the user.
pub fn tool_name(t: Tool) -> &'static str {
    match t {
        Tool::Cut => "cut",
        Tool::Draw => "draw",
        Tool::Emboss => "emboss",
        Tool::Foil => "foil",
    }
}

/// Layer-name hints: "draw" (or "pen"), "emboss" and "foil" pick that tool; anything else cuts.
pub fn tool_for(layer: &str) -> Tool {
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
    layers: Vec<LayerInfo>,
    by_tool: [usize; 4],
    open_paths: usize,
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
                let before = self.pieces.len();
                for c in children {
                    self.node(c, t);
                }
                let name = n.name.clone().unwrap_or_default();
                self.layers.push(LayerInfo { name, tool: tool_name(t), pieces: self.pieces.len() - before });
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
        self.open_paths += contours.iter().filter(|c| !c.closed).count();
        let slot = match tool {
            Tool::Cut => 0,
            Tool::Draw => 1,
            Tool::Emboss => 2,
            Tool::Foil => 3,
        };
        self.by_tool[slot] += 1;
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

/// What the walk found, and the size of the artboard and of the pieces that fall off the mat.
struct Found {
    walk: Walk,
    artboard_mm: (f64, f64),
    off_mat: usize,
}

fn walk_document(doc: &Document, opts: &FcmOptions) -> Result<Found, String> {
    let board = doc.artboards.get(opts.artboard).ok_or_else(|| format!("no artboard {}", opts.artboard.saturating_add(1)))?;
    let r: Rect = board.rect;
    let (w, h) = (r.width() * MM_PER_PT, r.height() * MM_PER_PT);
    let mut walk = Walk { pieces: vec![], warnings: vec![], origin: (r.x0, r.y0), layers: vec![], by_tool: [0; 4], open_paths: 0 };
    if w > opts.mat_mm.0 + 0.5 || h > opts.mat_mm.1 + 0.5 {
        walk.warn(&format!("the artboard ({w:.1} x {h:.1} mm) is larger than the {:.1} x {:.1} mm cutting area", opts.mat_mm.0, opts.mat_mm.1));
    }
    for l in &doc.layers {
        walk.node(l, Tool::Cut);
    }
    // pieces whose box leaves the cutting area
    let off_mat = walk
        .pieces
        .iter()
        .filter(|p| p.position_mm.0 < 0.0 || p.position_mm.1 < 0.0 || p.position_mm.0 > opts.mat_mm.0 || p.position_mm.1 > opts.mat_mm.1)
        .count();
    if off_mat > 0 {
        walk.warn(&format!("{off_mat} piece(s) are centred outside the cutting area"));
    }
    Ok(Found { walk, artboard_mm: (w, h), off_mat })
}

pub fn export(doc: &Document, opts: &FcmOptions) -> Result<FcmOutput, String> {
    let Found { walk, .. } = walk_document(doc, opts)?;
    if walk.pieces.is_empty() {
        return Err("nothing to cut: no visible paths on the layers".into());
    }
    let design = Design { mat_width_mm: opts.mat_mm.0, mat_height_mm: opts.mat_mm.1, pieces: walk.pieces };
    let bytes = fcm_core::write(&design.to_fcm());
    Ok(FcmOutput { bytes, warnings: walk.warnings })
}

/// What [`export`] would write for `doc`, without writing anything. An empty document is not an error
/// here: it reports no pieces.
pub fn summarize(doc: &Document, opts: &FcmOptions) -> Result<FcmSummary, String> {
    let Found { walk, artboard_mm, off_mat } = walk_document(doc, opts)?;
    let by_tool = [("cut", walk.by_tool[0]), ("draw", walk.by_tool[1]), ("emboss", walk.by_tool[2]), ("foil", walk.by_tool[3])];
    Ok(FcmSummary {
        artboard_mm,
        cutting_area_mm: opts.mat_mm,
        pieces: walk.pieces.len(),
        by_tool,
        layers: walk.layers,
        open_paths: walk.open_paths,
        off_mat,
        warnings: walk.warnings,
    })
}
