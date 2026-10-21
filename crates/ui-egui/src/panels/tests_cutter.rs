//! The Cutter panel, drawn headless: the placed mat scan, what will be cut, and the Export button.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::tests_labels::painted_text;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app
}

fn with_rectangle() -> VectorcraftApp {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 60})).unwrap();
    app
}

fn panel(app: &mut VectorcraftApp) -> String {
    painted_text(app, super::cutter::show)
}

fn grey_png(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("vc-cutter-panel-{}-{name}.png", std::process::id()));
    image::RgbImage::from_pixel(120, 160, image::Rgb([200, 200, 200])).save(&path).unwrap();
    path
}

#[test]
fn without_a_document_the_panel_says_so() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert!(panel(&mut app).contains("No document"));
}

#[test]
fn an_empty_document_has_nothing_to_cut_and_no_export_is_offered_as_a_piece_count() {
    let mut app = app();
    let text = panel(&mut app);
    assert!(text.contains("Nothing to cut yet"), "{text}");
    assert!(text.contains("Import scan"), "{text}");
    assert!(!text.contains("Pieces"), "{text}");
}

#[test]
fn a_drawn_path_shows_as_a_piece_to_cut_on_its_layer() {
    let mut app = with_rectangle();
    let text = panel(&mut app);
    assert!(text.contains("Pieces"), "{text}");
    assert!(text.contains("Cut"), "{text}");
    assert!(text.contains("Export FCM"), "{text}");
    assert!(!text.contains("Nothing to cut yet"), "{text}");
}

#[test]
fn a_layer_named_draw_is_shown_as_the_draw_tool() {
    let mut app = with_rectangle();
    app.run("layer.setProps", json!({"name": "Pen lines"})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("Draw"), "{text}");
    assert!(text.contains("Pen lines"), "{text}");
}

#[test]
fn the_panel_follows_edits_to_the_document() {
    let mut app = with_rectangle();
    assert!(panel(&mut app).contains("Pieces"));
    app.run("select.all", json!({})).unwrap();
    app.run("edit.clear", json!({})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("Nothing to cut yet"), "after deleting the path: {text}");
}

#[test]
fn a_placed_scan_shows_its_size_and_is_removed_with_its_layer() {
    let png = grey_png("a");
    let mut app = with_rectangle();
    app.run("document.placeMatScan", json!({"path": png.to_string_lossy()})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("120 x 160 px"), "{text}");
    assert!(text.contains("Remove scan") && !text.contains("Import scan"), "{text}");
    // the scan is a template layer: still exactly one piece to cut
    assert!(text.contains("Pieces"), "{text}");
    let layer = app.session.active().unwrap().doc.layers.iter().find(|l| l.name.as_deref() == Some("Mat scan")).unwrap().id;
    app.run("layer.delete", json!({"ids": [layer.0]})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("Import scan") && !text.contains("Remove scan"), "{text}");
    std::fs::remove_file(png).ok();
}

#[test]
fn a_piece_off_the_mat_is_warned_about() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 5000, "y": 20, "width": 80, "height": 60})).unwrap();
    let text = panel(&mut app);
    assert!(text.contains("outside the cutting area") || text.contains("larger than"), "{text}");
}

#[test]
fn the_panel_is_listed_with_the_other_panels_and_has_an_icon() {
    assert!(crate::state::all_panels().any(|(id, label)| id == "cutter" && label == "Cutter"));
    assert!(crate::state::ICON_PANEL_GROUPS.iter().any(|g| g.contains(&"cutter")), "shown as an icon in the dock");
    assert!(crate::state::ICON_PANELS.iter().any(|&(id, _, icon)| id == "cutter" && icon == "scissors"));
}
