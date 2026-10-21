//! Cutter panel: everything for a cutting machine in one place. The mat scan the machine saved to USB
//! (placed at mat coordinates on a locked template layer, so artwork lines up with the real mat), what
//! an export would write (pieces per tool, which tool each layer gets, anything off the mat), and
//! the Export button (the Export As dialog with FCM chosen).
//!
//! Everything runs through commands: `document.placeMatScan`, `document.cutterSummary`,
//! `layer.setProps` / `layer.delete` for the scan's layer, and `file.exportAs`.
//!
//! The panel's own text is English only for now (not wrapped in `tl!`): it needs translating, in every
//! language that claims complete menus, before this goes upstream.

use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use super::doc_info::row;
use super::{pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets;

pub const ID: &str = "cutter";
/// The layer `document.placeMatScan` puts the scan on.
const SCAN_LAYER: &str = "Mat scan";
/// How strongly the template layer shows the scan when it is placed (a template layer's default).
const DEFAULT_SCAN_PERCENT: f32 = 50.0;

/// `document.cutterSummary` and what it was fetched for: (document uid, revision).
#[derive(Clone, Default)]
struct Cache {
    key: Option<(u64, u64)>,
    summary: Option<Value>,
    error: String,
}

/// The placed scan: its layer and its size in pixels.
struct Scan {
    layer: NodeId,
    px: Option<(u32, u32)>,
}

fn scan(app: &VectorcraftApp) -> Option<Scan> {
    let d = app.session.active()?;
    let l = d.doc.layers.iter().find(|l| l.name.as_deref() == Some(SCAN_LAYER) && matches!(l.kind, NodeKind::Layer { .. }))?;
    let px = match &l.kind {
        NodeKind::Layer { children, .. } => children.iter().find_map(|c| match &c.kind {
            NodeKind::Image(i) => Some((i.width, i.height)),
            _ => None,
        }),
        _ => None,
    };
    Some(Scan { layer: l.id, px })
}

/// What an export would write, asked again only when the document changes.
fn summary(app: &mut VectorcraftApp, ctx: &egui::Context) -> Result<Value, String> {
    let (uid, revision) = app.session.active().map(|d| (d.uid, d.revision)).ok_or_else(String::new)?;
    let mut c: Cache = pstate(ctx, "cutter-cache");
    if c.key != Some((uid, revision)) {
        match app.session.execute("document.cutterSummary", &json!({})) {
            Ok(v) => {
                c.summary = Some(v);
                c.error.clear();
            }
            Err(e) => {
                c.summary = None;
                c.error = e.to_string();
            }
        }
        c.key = Some((uid, revision));
        set_pstate(ctx, "cutter-cache", c.clone());
    }
    c.summary.ok_or(c.error)
}

fn tool_label(tool: &str) -> &'static str {
    match tool {
        "draw" => "Draw",
        "emboss" => "Emboss",
        "foil" => "Foil",
        _ => "Cut",
    }
}

fn mat_scan_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    widgets::section_header(ui, "Mat scan");
    row(ui, "Mat", "Brother 12 x 12 in (304.8 mm)".into());
    match scan(app) {
        Some(s) => {
            let size = s.px.map_or_else(|| "placed".to_string(), |(w, h)| format!("{w} x {h} px"));
            row(ui, "Scan", size);
            let mut pct: f32 = pstate(ui.ctx(), "cutter-scan-percent");
            if pct == 0.0 {
                pct = DEFAULT_SCAN_PERCENT;
            }
            ui.horizontal(|ui| {
                widgets::field_label(ui, "Scan opacity");
                if widgets::slider(ui, egui::Slider::new(&mut pct, 1.0..=100.0).suffix("%")).changed() {
                    set_pstate(ui.ctx(), "cutter-scan-percent", pct);
                    crate::menus::invoke(app, "layer.setProps", json!({ "ids": [s.layer.0], "dimImages": pct.round() as u32 }));
                }
            });
            if widgets::secondary_button(ui, "Remove scan").clicked() {
                crate::menus::invoke(app, "layer.delete", json!({ "ids": [s.layer.0] }));
            }
            widgets::dim_label(ui, "The scan is a locked template layer: it is not cut or exported.");
        }
        None => {
            widgets::dim_label(ui, "Scan the mat to USB on the machine, then place the picture here so your art lines up with it.");
            row(ui, "Profile", "built-in (CM900)".into());
            ui.horizontal_wrapped(|ui| {
                if widgets::secondary_button(ui, "Place scan…").clicked() {
                    crate::menus::invoke(app, "document.placeMatScan", json!({}));
                }
                if widgets::secondary_button(ui, "Place with profile file…").clicked() {
                    crate::menus::invoke(app, "document.placeMatScan", json!({ "pickProfile": true }));
                }
            });
        }
    }
}

fn export_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    widgets::section_header(ui, "What will be cut");
    let s = match summary(app, ui.ctx()) {
        Ok(s) => s,
        Err(e) => {
            widgets::dim_label(ui, &e);
            return;
        }
    };
    let pieces = s["pieces"].as_u64().unwrap_or(0);
    if pieces == 0 {
        widgets::dim_label(ui, "Nothing to cut yet: draw paths on a visible layer.");
    } else {
        row(ui, "Pieces", pieces.to_string());
        for b in s["byTool"].as_array().into_iter().flatten() {
            let n = b["pieces"].as_u64().unwrap_or(0);
            if n > 0 {
                row(ui, tool_label(b["tool"].as_str().unwrap_or("cut")), n.to_string());
            }
        }
        if s["openPaths"].as_u64().unwrap_or(0) > 0 {
            row(ui, "Open paths", s["openPaths"].to_string());
        }
        widgets::subheader(ui, "Layers");
        for l in s["layers"].as_array().into_iter().flatten() {
            let name = l["name"].as_str().filter(|n| !n.is_empty()).unwrap_or("(unnamed layer)");
            let text = format!("{}, {}", tool_label(l["tool"].as_str().unwrap_or("cut")), l["pieces"].as_u64().unwrap_or(0));
            row(ui, name, text);
        }
    }
    for w in s["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        ui.label(egui::RichText::new(w).size(12.0).color(t.warning));
    }
    widgets::dim_label(ui, "A layer's name picks the tool: draw or pen, emboss, foil. Any other name cuts.");
    ui.add_space(6.0);
    if widgets::primary_button_enabled(ui, "Export FCM…", pieces > 0).clicked() {
        crate::menus::invoke(app, "file.exportAs", json!({ "format": "fcm" }));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        widgets::dim_label(ui, tl!("No document"));
        return;
    }
    egui::ScrollArea::vertical().show(ui, |ui| {
        mat_scan_section(app, ui);
        widgets::divider(ui);
        export_section(app, ui);
    });
}
