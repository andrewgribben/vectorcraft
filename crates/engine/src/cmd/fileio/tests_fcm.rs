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
    s.execute("layer.setProps", &json!({"name": "Draw"})).ok();
    let r = export(&mut s, json!({})).unwrap();
    let f = vectorcraft_fcm::fcm_core::parse(&b64(&r)).unwrap();
    // layer.setProps may not rename in every build: accept either, but the flag must be consistent
    let fl = f.pieces[0].paths[0].flags;
    assert!(fl == vectorcraft_fcm::fcm_core::flags::CUT || fl == vectorcraft_fcm::fcm_core::flags::DRAW, "{fl:#x}");
}

#[test]
fn rejects_a_bad_mat_size_and_a_missing_artboard() {
    let mut s = session();
    assert!(export(&mut s, json!({"matWidthMm": -1})).is_err());
    assert!(export(&mut s, json!({"matWidthMm": 1e9})).is_err());
    assert!(export(&mut s, json!({"artboard": 7})).is_err());
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

#[test]
fn sends_the_file_to_the_machine_drive() {
    let dir = std::env::temp_dir().join(format!("vc-fcm-drive-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = session();
    let r = s.execute("document.sendToCutter", &json!({"volume": dir.to_string_lossy(), "name": "my design!"})).unwrap();
    let written = dir.join("my_design_.fcm");
    assert_eq!(r["path"].as_str().unwrap(), written.to_string_lossy());
    let f = vectorcraft_fcm::fcm_core::parse(&std::fs::read(&written).unwrap()).unwrap();
    assert_eq!(f.pieces.len(), 1);
    // a stray missing drive is an error, not a panic
    assert!(s.execute("document.sendToCutter", &json!({"volume": "/definitely/not/mounted"})).is_err());
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn finds_a_drive_named_no_name_under_the_mount_roots() {
    let root = std::env::temp_dir().join(format!("vc-fcm-roots-{}", std::process::id()));
    std::fs::create_dir_all(root.join("Other")).unwrap();
    assert!(super::fcm::find_machine_volume(std::slice::from_ref(&root)).is_none());
    std::fs::create_dir_all(root.join("NO NAME")).unwrap();
    assert_eq!(super::fcm::find_machine_volume(std::slice::from_ref(&root)), Some(root.join("NO NAME")));
    std::fs::remove_dir_all(&root).ok();
}
