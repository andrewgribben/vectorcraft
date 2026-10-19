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
    vec![cmd!(
        "document.sendToCutter",
        "Send to ScanNCut",
        [],
        None,
        "{volume?: \"/Volumes/NO NAME\" (the cutter's drive; default: found among the mounted drives), name?: file name without extension (default: the document's), …document.export options for fcm} → {path, bytes, warnings, volume}; writes the cutting file straight onto the machine's USB drive, where the machine's own screen picks it up. Same as document.export {format: fcm, path} with the drive found for you",
        has_doc,
        send_to_cutter
    )]
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
