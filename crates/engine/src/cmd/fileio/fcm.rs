//! ScanNCut `.fcm` export (`document.export {format: fcm}`): one cutting file for the chosen artboard,
//! which is the mat (its top-left corner is the mat's top-left corner).

use std::path::{Path, PathBuf};

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

pub(super) fn encode(doc: &vectorcraft_doc::Document, p: &Value) -> Result<Encoded> {
    let q: Params = if p.is_object() { Params::deserialize(p).map_err(|e| bad(C, format!("FCM options: {e}")))? } else { Params::default() };
    let d = vectorcraft_fcm::FcmOptions::default();
    let mat = (q.mat_width_mm.unwrap_or(d.mat_mm.0), q.mat_height_mm.unwrap_or(d.mat_mm.1));
    if !(mat.0.is_finite() && mat.1.is_finite() && mat.0 > 0.0 && mat.1 > 0.0 && mat.0 <= 2000.0 && mat.1 <= 2000.0) {
        return Err(bad(C, "FCM matWidthMm/matHeightMm must be positive numbers up to 2000"));
    }
    let out = vectorcraft_fcm::export(doc, &vectorcraft_fcm::FcmOptions { artboard: q.artboard.unwrap_or(0), mat_mm: mat }).map_err(|e| bad(C, e))?;
    Ok(Encoded { warnings: out.warnings, ..Encoded::one(out.bytes) })
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.placeMatScan",
            "Place Mat Scan",
            [],
            None,
            "{path (the scan JPEG the machine saved to USB, IMG/000001.jpg), profile?: {pxPerMmX, pxPerMmY, originPxX, originPxY} (this machine's scan calibration; default: the measured CM900 one), matWidthMm?: 304.8, matHeightMm?: 304.8 (the Brother 12 x 12 in mat), resizeArtboard?: true (artboard 1 becomes the mat, top-left at 0, 0)} → {id, layer, xMm, yMm, widthMm, heightMm}; places the scan on a locked template layer \"Mat scan\" so that a point in the picture lies at the same position on the artboard as on the mat. Template layers are not exported to the cutter, so your art lines up with what is on the mat. No path: error",
            has_doc,
            place_mat_scan
        ),
        cmd!(
            "document.sendToCutter",
            "Send to ScanNCut",
            [],
            None,
            "{volume?: \"/Volumes/NO NAME\" (the cutter's drive; default: found among the mounted drives), name?: file name without extension (default: the document's), …document.export options for fcm} → {path, bytes, warnings, volume}; writes the cutting file straight onto the machine's USB drive, where the machine's own screen picks it up. Same as document.export {format: fcm, path} with the drive found for you",
            has_doc,
            send_to_cutter
        ),
    ]
}

/// The drive names a ScanNCut presents (lower case).
const MACHINE_VOLUMES: &[&str] = &["no name"];

/// Mount roots to look in: macOS, Linux desktops.
fn mount_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/Volumes")];
    if let Some(user) = std::env::var_os("USER") {
        roots.push(Path::new("/media").join(&user));
        roots.push(Path::new("/run/media").join(&user));
    }
    roots
}

/// The first mounted drive that looks like a ScanNCut's, under `roots`.
pub(super) fn find_machine_volume(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else { continue };
        let mut found: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.is_dir() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| MACHINE_VOLUMES.contains(&n.to_ascii_lowercase().as_str()))
            })
            .collect();
        found.sort();
        if let Some(p) = found.into_iter().next() {
            return Some(p);
        }
    }
    None
}

/// A file name the machine's FAT drive takes: letters, digits, `-` and `_`, at most 24 characters.
fn safe_name(name: &str) -> String {
    let n: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(24).collect();
    if n.is_empty() { "design".to_string() } else { n }
}

fn send_to_cutter(s: &mut Session, p: &Value) -> Result<Value> {
    let c = "document.sendToCutter";
    let volume = match p.get("volume").and_then(Value::as_str) {
        Some(v) => PathBuf::from(v),
        None => find_machine_volume(&mount_roots())
            .ok_or_else(|| bad(c, "no ScanNCut drive found: connect the machine (it appears as a drive named NO NAME) or pass volume"))?,
    };
    if !volume.is_dir() {
        return Err(bad(c, format!("{} is not a mounted drive", volume.display())));
    }
    let base = match p.get("name").and_then(Value::as_str) {
        Some(n) => safe_name(n),
        None => safe_name(s.doc()?.doc.title.trim_end_matches(".fcm")),
    };
    let path = volume.join(format!("{base}.fcm"));
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    for k in ["volume", "name"] {
        if let Some(o) = q.as_object_mut() {
            o.remove(k);
        }
    }
    q["format"] = json!("fcm");
    q["path"] = json!(path.to_string_lossy());
    let mut r = super::export::export(s, &q)?;
    r["volume"] = json!(volume.to_string_lossy());
    Ok(r)
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
    let prof = match p.get("profile") {
        None | Some(Value::Null) => d,
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
    if p.get("resizeArtboard").and_then(Value::as_bool).unwrap_or(true) {
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
