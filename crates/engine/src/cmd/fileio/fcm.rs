//! ScanNCut `.fcm` export (`document.export {format: fcm}`): one cutting file for the chosen artboard,
//! which is the mat (its top-left corner is the mat's top-left corner).

use serde::Deserialize;
use serde_json::{Value, json};

use super::super::*;
use super::{Encoded, FormatOption};

const C: &str = "document.export";

/// The options `document.formats` lists for FCM.
pub(super) const OPTIONS: &[FormatOption] = &[
    FormatOption { name: "artboard", ty: "integer", default: "0", description: "the artboard that is the cutting mat (0-based)" },
    FormatOption { name: "matWidthMm", ty: "number", default: "296.67", description: "cutting area width in mm (12 x 12 inch mat by default)" },
    FormatOption { name: "matHeightMm", ty: "number", default: "298.8", description: "cutting area height in mm (12 x 12 inch mat by default)" },
];

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Params {
    artboard: Option<usize>,
    mat_width_mm: Option<f64>,
    mat_height_mm: Option<f64>,
}

/// The export options from the command params; the cutting area must be a sensible size.
fn options(c: &str, p: &Value) -> Result<vectorcraft_fcm::FcmOptions> {
    let q: Params = if p.is_object() { Params::deserialize(p).map_err(|e| bad(c, format!("FCM options: {e}")))? } else { Params::default() };
    let d = vectorcraft_fcm::FcmOptions::default();
    let mat = (q.mat_width_mm.unwrap_or(d.mat_mm.0), q.mat_height_mm.unwrap_or(d.mat_mm.1));
    if !(mat.0.is_finite() && mat.1.is_finite() && mat.0 > 0.0 && mat.1 > 0.0 && mat.0 <= 2000.0 && mat.1 <= 2000.0) {
        return Err(bad(c, "FCM matWidthMm/matHeightMm must be positive numbers up to 2000"));
    }
    Ok(vectorcraft_fcm::FcmOptions { artboard: q.artboard.unwrap_or(0), mat_mm: mat })
}

pub(super) fn encode(doc: &vectorcraft_doc::Document, p: &Value) -> Result<Encoded> {
    let opts = options(C, p)?;
    let out = vectorcraft_fcm::export(doc, &opts).map_err(|e| bad(C, e))?;
    Ok(Encoded { warnings: out.warnings, ..Encoded::one(out.bytes) })
}

fn cutter_summary(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "document.cutterSummary";
    let opts = options(c, p)?;
    let sum = vectorcraft_fcm::summarize(&s.doc()?.doc, &opts).map_err(|e| bad(c, e))?;
    Ok(json!({
        "pieces": sum.pieces,
        "byTool": sum.by_tool.iter().map(|(t, n)| json!({"tool": t, "pieces": n})).collect::<Vec<_>>(),
        "layers": sum.layers.iter().map(|l| json!({"name": l.name, "tool": l.tool, "pieces": l.pieces})).collect::<Vec<_>>(),
        "openPaths": sum.open_paths,
        "offMat": sum.off_mat,
        "artboardMm": [sum.artboard_mm.0, sum.artboard_mm.1],
        "cuttingAreaMm": [sum.cutting_area_mm.0, sum.cutting_area_mm.1],
        "warnings": sum.warnings,
    }))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "document.cutterSummary",
            "Cutter Summary",
            [],
            None,
            "{artboard?: 0, matWidthMm?: 296.67, matHeightMm?: 298.8} → {pieces, byTool: [{tool: cut|draw|emboss|foil, pieces}], layers: [{name, tool, pieces}], openPaths, offMat, artboardMm: [w, h], cuttingAreaMm: [w, h], warnings}; what an FCM export would write for this document, without writing a file: pieces per tool (a layer's name picks the tool: draw or pen, emboss, foil, else cut), pieces centred off the cutting area, open paths and the export's warnings. Template layers (such as the placed mat scan) are not counted",
            has_doc,
            cutter_summary
        ),
        cmd!(
            "document.placeMatScan",
            "Import Scan",
            [],
            None,
            "{path (the scan JPEG the machine saved to USB, IMG/000001.jpg), profile?: {pxPerMmX, pxPerMmY, originPxX, originPxY} | profilePath?: a JSON file with those four numbers (this machine's scan calibration; default: the measured CM900 one), matWidthMm?: 304.8, matHeightMm?: 304.8 (the Brother 12 x 12 in mat), resizeArtboard?: false (true: artboard 1 becomes the mat, top-left at 0, 0; by default the artboard is left as it is, and the scan is placed from its top-left corner, which is the mat's top-left corner)} → {id, layer, xMm, yMm, widthMm, heightMm}; places the scan on a locked template layer \"Mat scan\" so that a point in the picture lies at the same position on the artboard as on the mat. Template layers are not exported to the cutter, so your art lines up with what is on the mat. No path: error",
            has_doc,
            place_mat_scan
        ),
    ]
}

/// A finite number param, or `default`.
fn num(c: &str, p: &Value, key: &str, default: f64) -> Result<f64> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(c, format!("{key} must be a number"))),
    }
}

fn profile(c: &str, p: &Value) -> Result<vectorcraft_fcm::scan::MatScanProfile> {
    let d = vectorcraft_fcm::scan::MatScanProfile::default();
    let from_file;
    let given = p.get("profile").filter(|v| !v.is_null());
    let src = match (given, str_param(p, "profilePath")) {
        (Some(_), Some(_)) => return Err(bad(c, "give profile or profilePath, not both")),
        (None, Some(path)) => {
            let bytes = super::read_file(path)?;
            if bytes.len() > 64 * 1024 {
                return Err(bad(c, "the profile file is too big for a scan profile"));
            }
            from_file = serde_json::from_slice::<Value>(&bytes).map_err(|e| bad(c, format!("profile file {path}: {e}")))?;
            Some(&from_file)
        }
        (v, None) => v,
    };
    let prof = match src {
        None => d,
        Some(v) if v.is_object() => vectorcraft_fcm::scan::MatScanProfile {
            px_per_mm_x: num(c, v, "pxPerMmX", d.px_per_mm_x)?,
            px_per_mm_y: num(c, v, "pxPerMmY", d.px_per_mm_y)?,
            origin_px_x: num(c, v, "originPxX", d.origin_px_x)?,
            origin_px_y: num(c, v, "originPxY", d.origin_px_y)?,
        },
        Some(_) => return Err(bad(c, "profile must be an object")),
    };
    prof.validate().map_err(|e| bad(c, e))?;
    Ok(prof)
}

fn place_mat_scan(s: &mut Session, p: &Value) -> Result<Value> {
    use vectorcraft_fcm::scan::MM_PER_PT;
    let c = "document.placeMatScan";
    let path = str_param(p, "path").ok_or_else(|| bad(c, "path: the scan JPEG the machine saved to USB"))?;
    let prof = profile(c, p)?;
    let mat = (num(c, p, "matWidthMm", 304.8)?, num(c, p, "matHeightMm", 304.8)?);
    if !(mat.0 > 0.0 && mat.1 > 0.0 && mat.0 <= 2000.0 && mat.1 <= 2000.0) {
        return Err(bad(c, "matWidthMm/matHeightMm must be positive numbers up to 2000"));
    }
    let pt = |mm: f64| mm / MM_PER_PT;
    if p.get("resizeArtboard").and_then(Value::as_bool).unwrap_or(false) {
        s.execute("artboard.setProps", &json!({"index": 0, "x": 0, "y": 0, "width": pt(mat.0), "height": pt(mat.1)}))?;
    }
    let artboard = s.doc()?.doc.artboards.first().map(|a| a.rect).ok_or_else(|| bad(c, "the document has no artboard"))?;
    // Its own layer, unlocked until the picture is in place, then a template (locked, not exported).
    let layer = s.execute("layer.new", &json!({"name": "Mat scan", "top": false}))?;
    let placed = s.execute("file.place", &json!({"path": path, "x": artboard.x0, "y": artboard.y0}))?;
    let id = placed["ids"].as_array().and_then(|a| a.first()).and_then(Value::as_u64).ok_or_else(|| bad(c, "the scan was not placed"))?;
    let (w_px, h_px) = match s.doc()?.doc.node(vectorcraft_doc::NodeId(id)).map(|n| &n.kind) {
        Some(vectorcraft_doc::NodeKind::Image(img)) => (img.width, img.height),
        _ => return Err(bad(c, "the placed file is not a picture")),
    };
    let b = prof.placement(w_px, h_px);
    s.execute("select.set", &json!({"ids": [id]}))?;
    s.execute(
        "object.setBounds",
        &json!({
            "x": artboard.x0 + pt(b.x_mm),
            "y": artboard.y0 + pt(b.y_mm),
            "width": pt(b.width_mm),
            "height": pt(b.height_mm),
            "reference": 0,
            "proportional": false,
        }),
    )?;
    s.execute("layer.setProps", &json!({"id": layer["id"], "template": true}))?;
    Ok(json!({"id": id, "layer": layer["id"], "xMm": b.x_mm, "yMm": b.y_mm, "widthMm": b.width_mm, "heightMm": b.height_mm}))
}
