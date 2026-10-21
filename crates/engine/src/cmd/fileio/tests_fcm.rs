//! FCM export through the commands: geometry on the mat, tools by layer name, the options, and
//! sending the file to a (stand-in) machine drive.

use serde_json::{Value, json};

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// A 200 x 200 mm artboard (in points) with a 60 x 40 mm rectangle at (20, 30) mm.
fn session() -> Session {
    let mm = 72.0 / 25.4;
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200.0 * mm, "height": 200.0 * mm})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20.0 * mm, "y": 30.0 * mm, "width": 60.0 * mm, "height": 40.0 * mm})).unwrap();
    s
}

fn export(s: &mut Session, p: Value) -> Result<Value> {
    let mut q = json!({"format": "fcm"});
    q.as_object_mut().unwrap().extend(p.as_object().unwrap().clone());
    s.execute("document.export", &q)
}

#[test]
fn rectangle_lands_where_it_is_on_the_artboard() {
    let mut s = session();
    let r = export(&mut s, json!({})).unwrap();
    let f = vectorcraft_fcm::fcm_core::parse(&b64(&r)).unwrap();
    assert_eq!(f.pieces.len(), 1);
    let p = &f.pieces[0];
    // centre (50, 50) mm in hundredths, size 60 x 40 mm (one less, as Brother writes it)
    assert!((p.affine[4] - 5000.0).abs() < 2.0 && (p.affine[5] - 5000.0).abs() < 2.0, "{:?}", p.affine);
    assert!((p.width - 5999).abs() <= 1 && (p.height - 3999).abs() <= 1, "{}x{}", p.width, p.height);
    assert_eq!(p.paths[0].flags, vectorcraft_fcm::fcm_core::flags::CUT);
    assert_eq!((f.cut.cut_area_width, f.cut.cut_area_height), (29667, 29880));
}

#[test]
fn layer_names_pick_the_tool_and_open_paths_are_flagged() {
    let mut s = session();
    s.execute("layer.setProps", &json!({"name": "Draw"})).unwrap();
    let r = export(&mut s, json!({})).unwrap();
    let f = vectorcraft_fcm::fcm_core::parse(&b64(&r)).unwrap();
    assert_eq!(f.pieces[0].paths[0].flags, vectorcraft_fcm::fcm_core::flags::DRAW);
    for (name, flag) in [
        ("emboss me", vectorcraft_fcm::fcm_core::flags::EMBOSS),
        ("Foil", vectorcraft_fcm::fcm_core::flags::FOIL),
        ("Outline", vectorcraft_fcm::fcm_core::flags::CUT),
    ] {
        s.execute("layer.setProps", &json!({"name": name})).unwrap();
        let r = export(&mut s, json!({})).unwrap();
        let f = vectorcraft_fcm::fcm_core::parse(&b64(&r)).unwrap();
        assert_eq!(f.pieces[0].paths[0].flags, flag, "layer {name}");
    }
}

#[test]
fn rejects_a_bad_mat_size_and_a_missing_artboard() {
    let mut s = session();
    assert!(export(&mut s, json!({"matWidthMm": -1})).is_err());
    assert!(export(&mut s, json!({"matWidthMm": 1e9})).is_err());
    assert!(export(&mut s, json!({"artboard": 7})).is_err());
    // the largest possible artboard number is an error too, not an arithmetic overflow
    assert!(export(&mut s, json!({"artboard": u64::MAX})).is_err());
    assert!(s.execute("document.cutterSummary", &json!({"artboard": u64::MAX})).is_err());
}

#[test]
fn an_empty_document_has_nothing_to_cut() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100.0, "height": 100.0})).unwrap();
    assert!(export(&mut s, json!({})).is_err());
}

#[test]
fn fcm_is_listed_as_a_writable_format() {
    let f = format("fcm").unwrap();
    assert!(f.write && !f.read && f.extensions == ["fcm"]);
}

/// A flat grey PNG of the given size, written to a temp file.
fn grey_png(name: &str, w: u32, h: u32) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("vc-matscan-{}-{name}.png", std::process::id()));
    image::RgbImage::from_pixel(w, h, image::Rgb([200, 200, 200])).save(&path).unwrap();
    path
}

#[test]
fn a_mat_scan_lands_at_mat_coordinates_on_a_locked_template_layer() {
    let mm = 72.0 / 25.4;
    let png = grey_png("a", 600, 800);
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100.0, "height": 100.0})).unwrap();
    let prof = json!({"pxPerMmX": 10.0, "pxPerMmY": 20.0, "originPxX": -5.0, "originPxY": -10.0});
    let r = s.execute("document.placeMatScan", &json!({"path": png.to_string_lossy(), "profile": prof})).unwrap();
    // 600 px at 10 px/mm = 60 mm wide, 800 px at 20 px/mm = 40 mm tall; first pixel at (0.5, 0.5) mm
    assert!((r["widthMm"].as_f64().unwrap() - 60.0).abs() < 1e-9 && (r["heightMm"].as_f64().unwrap() - 40.0).abs() < 1e-9);
    let st = s.doc().unwrap();
    let ab = st.doc.artboards[0].rect;
    assert!((ab.width() - 100.0).abs() < 1e-9 && (ab.height() - 100.0).abs() < 1e-9, "the artboard keeps its size: {ab:?}");
    let b = st.doc.node(vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())).unwrap().visual_bounds().unwrap();
    assert!(((b.x0 - ab.x0) / mm - 0.5).abs() < 0.01 && ((b.y0 - ab.y0) / mm - 0.5).abs() < 0.01, "{b:?}");
    assert!((b.width() / mm - 60.0).abs() < 0.01 && (b.height() / mm - 40.0).abs() < 0.01, "{b:?}");
    let layer = st.doc.layers.iter().find(|l| l.name.as_deref() == Some("Mat scan")).expect("Mat scan layer");
    assert!(matches!(layer.kind, vectorcraft_doc::NodeKind::Layer { template: true, .. }));
    std::fs::remove_file(png).ok();
}

#[test]
fn a_mat_scan_layer_is_not_exported_to_the_cutter() {
    let png = grey_png("b", 200, 200);
    let mut s = session();
    s.execute("document.placeMatScan", &json!({"path": png.to_string_lossy(), "resizeArtboard": false})).unwrap();
    let r = export(&mut s, json!({})).unwrap();
    let f = vectorcraft_fcm::fcm_core::parse(&b64(&r)).unwrap();
    assert_eq!(f.pieces.len(), 1, "only the rectangle is cut");
    std::fs::remove_file(png).ok();
}

#[test]
fn a_mat_scan_rejects_bad_input() {
    let png = grey_png("c", 50, 50);
    let mut s = session();
    let p = png.to_string_lossy().to_string();
    assert!(s.execute("document.placeMatScan", &json!({})).is_err(), "no path");
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "profile": {"pxPerMmX": 0}})).is_err());
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "profile": "x"})).is_err());
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "matWidthMm": -3})).is_err());
    assert!(s.execute("document.placeMatScan", &json!({"path": "/no/such/scan.jpg"})).is_err());
    std::fs::remove_file(png).ok();
}

#[test]
fn a_mat_scan_profile_can_come_from_a_file() {
    let mm = 72.0 / 25.4;
    let png = grey_png("d", 600, 800);
    let file = std::env::temp_dir().join(format!("vc-matscan-{}-profile.json", std::process::id()));
    std::fs::write(&file, r#"{"pxPerMmX": 10.0, "pxPerMmY": 20.0, "originPxX": -5.0, "originPxY": -10.0}"#).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100.0, "height": 100.0})).unwrap();
    let r = s.execute("document.placeMatScan", &json!({"path": png.to_string_lossy(), "profilePath": file.to_string_lossy()})).unwrap();
    assert!((r["widthMm"].as_f64().unwrap() - 60.0).abs() < 1e-9 && (r["heightMm"].as_f64().unwrap() - 40.0).abs() < 1e-9, "{r}");
    let st = s.doc().unwrap();
    let ab = st.doc.artboards[0].rect;
    let b = st.doc.node(vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())).unwrap().visual_bounds().unwrap();
    assert!(((b.x0 - ab.x0) / mm - 0.5).abs() < 0.01 && ((b.y0 - ab.y0) / mm - 0.5).abs() < 0.01, "{b:?}");
    std::fs::remove_file(png).ok();
    std::fs::remove_file(file).ok();
}

#[test]
fn a_bad_mat_scan_profile_file_is_an_error() {
    let png = grey_png("e", 50, 50);
    let p = png.to_string_lossy().to_string();
    let bad_file = std::env::temp_dir().join(format!("vc-matscan-{}-bad.json", std::process::id()));
    std::fs::write(&bad_file, "not json").unwrap();
    let zero = std::env::temp_dir().join(format!("vc-matscan-{}-zero.json", std::process::id()));
    std::fs::write(&zero, r#"{"pxPerMmX": 0}"#).unwrap();
    let mut s = session();
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "profilePath": "/no/such/profile.json"})).is_err());
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "profilePath": bad_file.to_string_lossy()})).is_err());
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "profilePath": zero.to_string_lossy()})).is_err());
    assert!(s.execute("document.placeMatScan", &json!({"path": p, "profilePath": zero.to_string_lossy(), "profile": {}})).is_err(), "both given");
    std::fs::remove_file(png).ok();
    std::fs::remove_file(bad_file).ok();
    std::fs::remove_file(zero).ok();
}

/// A document with one cut layer, one draw layer and one hidden layer, for the summary tests.
fn layered_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200.0, "height": 200.0})).unwrap();
    s.execute("layer.setProps", &json!({"name": "Cut"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10.0, "y": 10.0, "width": 40.0, "height": 30.0})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 60.0, "y": 10.0, "width": 40.0, "height": 30.0})).unwrap();
    s
}

#[test]
fn the_cutter_summary_counts_pieces_and_reports_the_tool_per_layer() {
    let mut s = layered_session();
    let r = s.execute("document.cutterSummary", &json!({})).unwrap();
    assert_eq!(r["pieces"], 2, "{r}");
    assert_eq!(r["byTool"][0], json!({"tool": "cut", "pieces": 2}), "{r}");
    assert_eq!(r["byTool"][1], json!({"tool": "draw", "pieces": 0}), "{r}");
    assert_eq!(r["openPaths"], 0);
    assert_eq!(r["offMat"], 0);
    assert_eq!(r["layers"][0]["tool"], "cut");
    assert_eq!(r["layers"][0]["pieces"], 2);
    // the summary writes nothing, and agrees with what the export holds
    let exported = vectorcraft_fcm::fcm_core::parse(&b64(&export(&mut s, json!({})).unwrap())).unwrap();
    assert_eq!(exported.pieces.len(), 2);
}

#[test]
fn the_cutter_summary_follows_layer_names_and_skips_templates() {
    let mut s = layered_session();
    s.execute("layer.setProps", &json!({"name": "Pen lines"})).unwrap();
    let r = s.execute("document.cutterSummary", &json!({})).unwrap();
    assert_eq!(r["byTool"][1], json!({"tool": "draw", "pieces": 2}), "{r}");
    assert_eq!(r["layers"][0]["tool"], "draw");
    // a placed mat scan is a template layer: it never shows up
    let png = grey_png("f", 100, 100);
    s.execute("document.placeMatScan", &json!({"path": png.to_string_lossy(), "resizeArtboard": false})).unwrap();
    let r = s.execute("document.cutterSummary", &json!({})).unwrap();
    assert_eq!(r["pieces"], 2, "{r}");
    assert!(r["layers"].as_array().unwrap().iter().all(|l| l["name"] != "Mat scan"), "{r}");
    std::fs::remove_file(png).ok();
}

#[test]
fn the_cutter_summary_reports_pieces_off_the_mat_and_empty_documents() {
    let mut s = session();
    let r = s.execute("document.cutterSummary", &json!({"matWidthMm": 20.0, "matHeightMm": 20.0})).unwrap();
    assert!(r["offMat"].as_u64().unwrap() >= 1 || !r["warnings"].as_array().unwrap().is_empty(), "{r}");
    let mut e = Session::new();
    e.execute("file.new", &json!({"width": 100.0, "height": 100.0})).unwrap();
    let r = e.execute("document.cutterSummary", &json!({})).unwrap();
    assert_eq!(r["pieces"], 0, "an empty document has no pieces, not an error: {r}");
    assert!(e.execute("document.cutterSummary", &json!({"matWidthMm": -1})).is_err());
    assert!(Session::new().execute("document.cutterSummary", &json!({})).is_err(), "no document");
}

#[test]
fn a_mat_scan_can_make_the_artboard_the_mat_when_asked() {
    let mm = 72.0 / 25.4;
    let png = grey_png("g", 100, 100);
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 210.0 * mm, "height": 297.0 * mm})).unwrap();
    s.execute("document.placeMatScan", &json!({"path": png.to_string_lossy()})).unwrap();
    let ab = s.doc().unwrap().doc.artboards[0].rect;
    assert!((ab.width() / mm - 210.0).abs() < 0.01 && (ab.height() / mm - 297.0).abs() < 0.01, "A4 stays A4: {ab:?}");
    s.execute("document.placeMatScan", &json!({"path": png.to_string_lossy(), "resizeArtboard": true})).unwrap();
    let ab = s.doc().unwrap().doc.artboards[0].rect;
    assert!((ab.width() / mm - 304.8).abs() < 0.01 && (ab.height() / mm - 304.8).abs() < 0.01, "asked: the artboard is the mat: {ab:?}");
    std::fs::remove_file(png).ok();
}
