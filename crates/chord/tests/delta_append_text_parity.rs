use pi_chord::delta::{Op, Seg, apply, track};
use pi_chord::json::JsonValue;
use serde_json::json;

fn doc() -> JsonValue {
    json!({
        "title": "t",
        "content": [{ "type": "text", "text": "hello", "tags": ["a"] }],
        "meta": { "note": "n" },
    })
}

fn key(name: &str) -> Seg {
    Seg::Key(name.to_owned())
}

fn text_path() -> Vec<Seg> {
    vec![key("content"), Seg::Index(0), key("text")]
}

#[test]
fn records_an_append_op_and_the_appended_value() {
    let mut tracker = track(doc());
    let before = tracker.value().clone();
    let mut change = tracker.begin_change();
    change.append_text(&text_path(), " world").expect("append");
    let prepared = change.prepare(&tracker).expect("prepare");
    assert_eq!(prepared.ops(), [Op::Append(text_path(), " world".into())]);
    assert_eq!(
        apply(Some(before.clone()), prepared.ops())
            .expect("ops apply")
            .clone(),
        prepared.value().clone()
    );
    tracker.adopt(prepared).expect("adopt");
    assert_eq!(tracker.value()["content"][0]["text"], json!("hello world"));
    assert_eq!(before["content"][0]["text"], json!("hello"));
}

#[test]
fn joins_several_appends_to_one_path_in_one_change_next_to_other_changes() {
    let mut tracker = track(doc());
    let mut change = tracker.begin_change();
    change.append_text(&text_path(), " a").expect("append");
    change.state()["title"] = json!("new");
    change.append_text(&text_path(), " b").expect("append");
    change.append_text(&text_path(), "").expect("append");
    change.append_text(&text_path(), " c").expect("append");
    let prepared = change.prepare(&tracker).expect("prepare");
    let mut ops: Vec<String> = prepared.ops().iter().map(|op| format!("{op:?}")).collect();
    ops.sort();
    let mut expected = [
        format!("{:?}", Op::Append(text_path(), " a b c".into())),
        format!("{:?}", Op::Set(vec![key("title")], json!("new"))),
    ];
    expected.sort();
    assert_eq!(ops, expected);
    tracker.adopt(prepared).expect("adopt");
    assert_eq!(tracker.value()["content"][0]["text"], json!("hello a b c"));
}

#[test]
fn falls_back_to_the_string_comparison_when_the_same_path_is_also_assigned() {
    let tracker = track(doc());
    let mut change = tracker.begin_change();
    change.append_text(&text_path(), " x").expect("append");
    change.state()["content"][0]["text"] = json!("hello x y");
    let prepared = change.prepare(&tracker).expect("prepare");
    assert_eq!(prepared.ops(), [Op::Append(text_path(), " x y".into())]);

    let mut change = tracker.begin_change();
    change.append_text(&text_path(), " z").expect("append");
    change.state()["content"][0]["text"] = json!("replaced");
    let prepared = change.prepare(&tracker).expect("prepare");
    assert_eq!(prepared.ops(), [Op::Set(text_path(), json!("replaced"))]);
}

#[test]
fn appends_inside_a_block_the_same_change_inserted_or_replaced() {
    let mut tracker = track(doc());
    let mut change = tracker.begin_change();
    let content = &mut change.state()["content"];
    content
        .as_array_mut()
        .expect("content")
        .push(json!({ "type": "text", "text": "new", "tags": [] }));
    change
        .append_text(&[key("content"), Seg::Index(1), key("text")], " block")
        .expect("append");
    change.state()["content"][0] = json!({ "type": "text", "text": "other", "tags": [] });
    change.append_text(&text_path(), " one").expect("append");
    let prepared = change.prepare(&tracker).expect("prepare");
    tracker.adopt(prepared).expect("adopt");
    assert_eq!(
        tracker.value()["content"]
            .as_array()
            .expect("content")
            .iter()
            .map(|block| block["text"].clone())
            .collect::<Vec<_>>(),
        vec![json!("other one"), json!("new block")]
    );
}

#[test]
fn rejects_a_path_that_is_not_a_string() {
    let tracker = track(doc());
    let mut change = tracker.begin_change();
    assert!(change.append_text(&[key("content")], "x").is_err());
    assert!(change.append_text(&[key("missing")], "x").is_err());
    assert!(change.append_text(&[], "x").is_err());
    assert!(
        change
            .append_text(&[key("content"), Seg::Index(0), key("tags")], "x")
            .is_err()
    );
}

#[test]
fn an_append_after_the_key_was_reassigned_in_the_same_change_falls_back() {
    let tracker = track(doc());
    let mut change = tracker.begin_change();
    change.state()["meta"]["note"] = json!("note");
    change
        .append_text(&[key("meta"), key("note")], "x")
        .expect("append");
    let prepared = change.prepare(&tracker).expect("prepare");
    assert_eq!(
        prepared.ops(),
        [Op::Append(vec![key("meta"), key("note")], "otex".into())]
    );
}
