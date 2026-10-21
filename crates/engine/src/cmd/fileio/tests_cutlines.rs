//! `document.cutLines`: outlines around traced shapes, on their own layer.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::*;

fn mm(v: f64) -> f64 {
    v * 72.0 / 25.4
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": mm(300.0), "height": mm(300.0)})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) {
    s.execute("shape.rectangle", &json!({"x": mm(x), "y": mm(y), "width": mm(w), "height": mm(h)})).unwrap();
}

fn cut_lines(s: &mut Session, p: Value) -> Result<Value> {
    s.execute("select.all", &json!({})).unwrap();
    s.execute("document.cutLines", &p)
}

/// The geometric bounds of a new outline, in mm: (x0, y0, x1, y1).
fn bounds_mm(s: &Session, id: &Value) -> (f64, f64, f64, f64) {
    let n = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
    let b = match &n.kind {
        NodeKind::Path { path, .. } => path.bounds().unwrap(),
        other => panic!("not a path: {other:?}"),
    };
    let k = 25.4 / 72.0;
    (b.x0 * k, b.y0 * k, b.x1 * k, b.y1 * k)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.1
}

#[test]
fn an_outline_stands_off_the_shape_by_the_margin() {
    let mut s = session();
    rect(&mut s, 50.0, 50.0, 40.0, 30.0);
    let r = cut_lines(&mut s, json!({"marginMm": 2.0})).unwrap();
    assert_eq!(r["pieces"], 1, "{r}");
    let b = bounds_mm(&s, &r["ids"][0]);
    assert!(close(b.0, 48.0) && close(b.1, 48.0) && close(b.2, 92.0) && close(b.3, 82.0), "{b:?}");
}

#[test]
fn separate_shapes_get_separate_outlines_and_touching_ones_become_one() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 100.0, 10.0, 30.0, 30.0);
    assert_eq!(cut_lines(&mut s, json!({})).unwrap()["pieces"], 2);
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 30.0, 20.0, 30.0, 30.0);
    let r = cut_lines(&mut s, json!({})).unwrap();
    assert_eq!(r["pieces"], 1, "overlapping shapes are one sticker: {r}");
}

#[test]
fn the_outlines_are_on_a_cut_layer_the_fcm_export_reads_as_cut() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 100.0, 10.0, 30.0, 30.0);
    let r = cut_lines(&mut s, json!({})).unwrap();
    let sum = s.execute("document.cutterSummary", &json!({})).unwrap();
    let layer = sum["layers"].as_array().unwrap().iter().find(|l| l["name"] == "Cut lines").unwrap_or_else(|| panic!("{sum}"));
    assert_eq!((layer["tool"].as_str(), layer["pieces"].as_u64()), (Some("cut"), Some(2)), "{sum}");
    assert_eq!(r["layer"].as_u64().map(|_| ()), Some(()));
    // running it again adds to the same layer: one layer of that name
    cut_lines(&mut s, json!({})).unwrap();
    let n = s.doc().unwrap().doc.layers.iter().filter(|l| l.name.as_deref() == Some("Cut lines")).count();
    assert_eq!(n, 1);
}

#[test]
fn a_layer_name_can_pick_another_tool() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    cut_lines(&mut s, json!({"layer": "Draw lines"})).unwrap();
    let sum = s.execute("document.cutterSummary", &json!({})).unwrap();
    let l = sum["layers"].as_array().unwrap().iter().find(|l| l["name"] == "Draw lines").unwrap_or_else(|| panic!("{sum}"));
    assert_eq!(l["tool"], "draw", "{sum}");
}

#[test]
fn holes_are_filled_unless_asked_to_keep_them() {
    let build = || {
        let mut s = session();
        rect(&mut s, 20.0, 20.0, 60.0, 60.0);
        rect(&mut s, 40.0, 40.0, 20.0, 20.0);
        s.execute("select.all", &json!({})).unwrap();
        s.execute("object.compoundPath.make", &json!({})).unwrap();
        s
    };
    let mut s = build();
    let r = s.execute("document.cutLines", &json!({"marginMm": 1.0})).unwrap();
    assert_eq!(r["pieces"], 1, "{r}");
    let b = bounds_mm(&s, &r["ids"][0]);
    assert!(close(b.0, 19.0) && close(b.2, 81.0), "the outside edge only, grown by 1 mm: {b:?}");
    let mut s = build();
    let r = s.execute("document.cutLines", &json!({"marginMm": 1.0, "fillHoles": false})).unwrap();
    let n = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    match &n.kind {
        NodeKind::Compound { children, .. } => assert_eq!(children.len(), 2, "outside edge and the hole"),
        other => panic!("expected a compound path: {other:?}"),
    }
}

#[test]
fn specks_under_the_minimum_area_are_left_out() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 100.0, 10.0, 1.0, 1.0);
    let r = cut_lines(&mut s, json!({"marginMm": 0.0})).unwrap();
    assert_eq!((r["pieces"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(1)), "{r}");
    let mut s = session();
    rect(&mut s, 100.0, 10.0, 1.0, 1.0);
    assert!(cut_lines(&mut s, json!({"marginMm": 0.0})).is_err(), "nothing left to cut");
    assert!(cut_lines(&mut s, json!({"marginMm": 0.0, "minAreaMm2": 0.5})).is_ok());
}

#[test]
fn a_zero_margin_keeps_the_outline_on_the_shape() {
    let mut s = session();
    rect(&mut s, 50.0, 50.0, 40.0, 30.0);
    let r = cut_lines(&mut s, json!({"marginMm": 0})).unwrap();
    let b = bounds_mm(&s, &r["ids"][0]);
    assert!(close(b.0, 50.0) && close(b.2, 90.0) && close(b.3, 80.0), "{b:?}");
}

#[test]
fn outlines_that_run_into_each_other_are_counted() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 43.0, 10.0, 30.0, 30.0); // 3 mm apart: margins of 2 mm meet
    assert_eq!(cut_lines(&mut s, json!({"marginMm": 2.0})).unwrap()["overlaps"], 1);
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 100.0, 10.0, 30.0, 30.0);
    assert_eq!(cut_lines(&mut s, json!({"marginMm": 2.0})).unwrap()["overlaps"], 0);
}

#[test]
fn bad_input_is_an_error_not_a_panic() {
    let mut s = session();
    assert!(s.execute("document.cutLines", &json!({})).is_err(), "nothing selected");
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    assert!(cut_lines(&mut s, json!({"marginMm": -1})).is_err());
    assert!(cut_lines(&mut s, json!({"marginMm": 500})).is_err());
    assert!(cut_lines(&mut s, json!({"marginMm": "wide"})).is_err());
    assert!(cut_lines(&mut s, json!({"minAreaMm2": -3})).is_err());
    assert!(cut_lines(&mut s, json!({"layer": "x".repeat(200)})).is_err());
    assert!(Session::new().execute("document.cutLines", &json!({})).is_err(), "no document");
}

#[test]
fn it_is_one_undo_step_for_the_outlines() {
    let mut s = session();
    rect(&mut s, 10.0, 10.0, 30.0, 30.0);
    rect(&mut s, 100.0, 10.0, 30.0, 30.0);
    s.execute("select.all", &json!({})).unwrap();
    s.execute("layer.new", &json!({"name": "Cut lines", "top": true})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let before =
        s.doc().unwrap().doc.layers.iter().find(|l| l.name.as_deref() == Some("Cut lines")).map_or(0, |l| l.children().map_or(0, |c| c.len()));
    s.execute("document.cutLines", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let after =
        s.doc().unwrap().doc.layers.iter().find(|l| l.name.as_deref() == Some("Cut lines")).map_or(0, |l| l.children().map_or(0, |c| c.len()));
    assert_eq!(before, after, "one undo removes all the outlines");
}
