use std::sync::{Arc, Mutex};

use pi_chord::delta::{MAX_PENDING_OPS, Op, Seg, apply};
use pi_chord::json::JsonValue;
use pi_chord::watch::{
    Deliveries, FrameListener, NativeWatch, Replace, ValueListener, Watch, WatchEnd, WatchError,
};
use pi_rt::cancel::CancelToken;
use serde_json::json;

#[derive(Debug, PartialEq)]
struct Call {
    value: Option<JsonValue>,
    ops: Vec<Op>,
}

type Calls = Arc<Mutex<Vec<Call>>>;

fn calls() -> Calls {
    Arc::new(Mutex::new(Vec::new()))
}

fn collector(calls: &Calls) -> FrameListener<JsonValue> {
    let calls = Arc::clone(calls);
    Box::new(move |value, ops, _cancel| {
        calls.lock().expect("calls").push(Call {
            value: value.cloned(),
            ops: ops.to_vec(),
        });
        Ok(())
    })
}

fn json_watch(initial: JsonValue) -> (Watch<JsonValue>, Arc<Deliveries>) {
    let deliveries = Arc::new(Deliveries::default());
    let watch = Watch::new(Some(initial), Arc::clone(&deliveries));
    (watch, deliveries)
}

fn take(calls: &Calls) -> Vec<Call> {
    std::mem::take(&mut *calls.lock().expect("calls"))
}

fn text(path: &str) -> Vec<Op> {
    vec![Op::Set(vec![Seg::Key(path.into())], json!(1))]
}

#[test]
fn keeps_the_acquisition_value_until_start_then_delivers_the_frames_since_in_one_call() {
    let (watch, deliveries) = json_watch(json!({ "n": 0, "text": "" }));
    watch.advance(json!({ "n": 1, "text": "" }), text("n"));
    watch.advance(
        json!({ "n": 2, "text": "a" }),
        vec![
            Op::Set(vec![Seg::Key("n".into())], json!(2)),
            Op::Append(vec![Seg::Key("text".into())], "a".into()),
        ],
    );
    watch.advance(
        json!({ "n": 2, "text": "ab" }),
        vec![Op::Append(vec![Seg::Key("text".into())], "b".into())],
    );
    deliveries.drain();
    assert_eq!(watch.value(), Some(json!({ "n": 0, "text": "" })));

    let recorded = calls();
    watch.start(collector(&recorded)).expect("start");
    assert!(take(&recorded).is_empty());
    deliveries.drain();
    assert_eq!(
        take(&recorded),
        vec![Call {
            value: Some(json!({ "n": 2, "text": "ab" })),
            ops: vec![
                Op::Set(vec![Seg::Key("n".into())], json!(2)),
                Op::Append(vec![Seg::Key("text".into())], "ab".into()),
            ],
        }]
    );
    assert_eq!(watch.value(), Some(json!({ "n": 2, "text": "ab" })));
}

#[test]
fn never_invokes_the_listener_inline() {
    let (watch, deliveries) = json_watch(json!(0));
    let recorded = calls();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    assert!(take(&recorded).is_empty());
    deliveries.drain();
    assert_eq!(
        take(&recorded),
        vec![Call {
            value: Some(json!(1)),
            ops: vec![Op::Replace(json!(1))],
        }]
    );
}

#[test]
fn folds_frames_advanced_during_a_call_into_the_next_call() {
    let (watch, deliveries) = json_watch(json!({ "text": "" }));
    let values = Arc::new(Mutex::new(Vec::new()));
    let advanced = watch.clone();
    let recorded = Arc::clone(&values);
    watch
        .start(Box::new(move |value, ops, _cancel| {
            recorded
                .lock()
                .expect("values")
                .push(value.expect("value").clone());
            if ops.len() == 1 && value == Some(&json!({ "text": "a" })) {
                advanced.advance(
                    json!({ "text": "ab" }),
                    vec![Op::Append(vec![Seg::Key("text".into())], "b".into())],
                );
                advanced.advance(
                    json!({ "text": "abc" }),
                    vec![Op::Append(vec![Seg::Key("text".into())], "c".into())],
                );
            }
            Ok(())
        }))
        .expect("start");
    watch.advance(
        json!({ "text": "a" }),
        vec![Op::Append(vec![Seg::Key("text".into())], "a".into())],
    );
    deliveries.drain();
    assert_eq!(
        *values.lock().expect("values"),
        vec![json!({ "text": "a" }), json!({ "text": "abc" })]
    );
    assert_eq!(watch.value(), Some(json!({ "text": "abc" })));
}

#[test]
fn folded_ops_turn_the_previous_value_into_the_delivered_one() {
    let (watch, deliveries) = json_watch(json!({ "tokens": 0, "text": "" }));
    let replica = Arc::new(Mutex::new(json!({ "tokens": 0, "text": "" })));
    let applied = Arc::clone(&replica);
    watch
        .start(Box::new(move |value, ops, _cancel| {
            let mut replica = applied.lock().expect("replica");
            *replica = apply(Some(replica.clone()), ops).expect("ops apply");
            assert_eq!(Some(replica.clone()), value.cloned());
            Ok(())
        }))
        .expect("start");
    let mut value = json!({ "tokens": 0, "text": "" });
    for token in 1..=60 {
        value["tokens"] = json!(token);
        value["text"] = json!(format!("{}tok ", value["text"].as_str().unwrap_or("")));
        watch.advance(
            value.clone(),
            vec![
                Op::Set(vec![Seg::Key("tokens".into())], json!(token)),
                Op::Append(vec![Seg::Key("text".into())], "tok ".into()),
            ],
        );
        if token % 10 == 0 {
            deliveries.drain();
        }
    }
    deliveries.drain();
    assert_eq!(watch.value(), Some(value));
}

fn advance_distinct(watch: &Watch<JsonValue>, count: usize) -> JsonValue {
    let mut newest = json!(0);
    for index in 0..count {
        newest = json!(index);
        watch.advance(
            newest.clone(),
            vec![Op::Set(
                vec![Seg::Key("k".into()), Seg::Index(index)],
                json!(index),
            )],
        );
    }
    newest
}

#[test]
fn keeps_max_pending_ops_after_folding_and_replaces_the_root_past_them() {
    let (exact, deliveries) = json_watch(json!(0));
    advance_distinct(&exact, MAX_PENDING_OPS);
    let recorded = calls();
    exact.start(collector(&recorded)).expect("start");
    deliveries.drain();
    let taken = take(&recorded);
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].ops.len(), MAX_PENDING_OPS);

    let (over, deliveries) = json_watch(json!(0));
    let newest = advance_distinct(&over, MAX_PENDING_OPS + 1);
    let recorded = calls();
    over.start(collector(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(
        take(&recorded),
        vec![Call {
            value: Some(newest.clone()),
            ops: vec![Op::Replace(newest)],
        }]
    );
}

#[test]
fn never_folds_the_in_flight_call_into_an_overflow() {
    let (watch, deliveries) = json_watch(json!(0));
    let recorded = calls();
    let advancing = watch.clone();
    let delivered = Arc::clone(&recorded);
    watch
        .start(Box::new(move |value, ops, _cancel| {
            let mut calls = delivered.lock().expect("calls");
            calls.push(Call {
                value: value.cloned(),
                ops: ops.to_vec(),
            });
            if calls.len() == 1 {
                advance_distinct(&advancing, MAX_PENDING_OPS + 1);
            }
            Ok(())
        }))
        .expect("start");
    let newest = advance_distinct(&watch, MAX_PENDING_OPS + 1);
    deliveries.drain();
    let taken = take(&recorded);
    assert_eq!(taken.len(), 2);
    assert_eq!(taken[1].value, Some(newest.clone()));
    assert_eq!(taken[1].ops, vec![Op::Replace(newest)]);
}

#[test]
fn starts_folding_again_after_an_overflow_was_delivered() {
    let (watch, deliveries) = json_watch(json!(0));
    advance_distinct(&watch, MAX_PENDING_OPS + 1);
    let recorded = calls();
    watch.start(collector(&recorded)).expect("start");
    deliveries.drain();
    watch.advance(json!({ "n": 1 }), vec![Op::Replace(json!({ "n": 1 }))]);
    watch.advance(
        json!({ "n": 2 }),
        vec![Op::Set(vec![Seg::Key("n".into())], json!(2))],
    );
    deliveries.drain();
    let taken = take(&recorded);
    assert_eq!(
        taken.last(),
        Some(&Call {
            value: Some(json!({ "n": 2 })),
            ops: vec![
                Op::Replace(json!({ "n": 1 })),
                Op::Set(vec![Seg::Key("n".into())], json!(2)),
            ],
        })
    );
}

#[test]
fn delivers_the_retirement_frame_last_and_then_ends_retired() {
    let (watch, deliveries) = json_watch(json!(1));
    let recorded = calls();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(json!(2), vec![Op::Replace(json!(2))]);
    deliveries.drain();
    watch.retire();
    watch.advance(json!(3), vec![Op::Replace(json!(3))]);
    deliveries.drain();
    assert_eq!(
        take(&recorded),
        vec![
            Call {
                value: Some(json!(2)),
                ops: vec![Op::Replace(json!(2))],
            },
            Call {
                value: None,
                ops: vec![Op::Replace(JsonValue::Null)],
            },
        ]
    );
    assert_eq!(watch.value(), None);
    assert_eq!(watch.closed().expect("closed").reason(), "retired");
}

#[test]
fn drops_pending_frames_also_an_overflow_for_the_retirement() {
    for pending in [3, MAX_PENDING_OPS + 1] {
        let (watch, deliveries) = json_watch(json!(0));
        advance_distinct(&watch, pending);
        watch.retire();
        let recorded = calls();
        watch.start(collector(&recorded)).expect("start");
        deliveries.drain();
        assert_eq!(
            take(&recorded),
            vec![Call {
                value: None,
                ops: vec![Op::Replace(JsonValue::Null)],
            }]
        );
        assert_eq!(watch.closed().expect("closed").reason(), "retired");
    }
}

#[test]
fn ends_retired_only_on_its_sources_retirement_never_on_a_null_value() {
    let (watch, deliveries) = json_watch(json!(1));
    let recorded = calls();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(JsonValue::Null, vec![Op::Replace(JsonValue::Null)]);
    deliveries.drain();
    assert_eq!(
        take(&recorded),
        vec![Call {
            value: Some(JsonValue::Null),
            ops: vec![Op::Replace(JsonValue::Null)],
        }]
    );
    assert!(watch.closed().is_none());
    assert_eq!(watch.terminate(WatchEnd::Stopped).reason(), "stopped");
}

#[test]
fn discards_a_retirement_still_pending_when_the_source_ends() {
    let (watch, deliveries) = json_watch(json!(1));
    watch.retire();
    watch.terminate(WatchEnd::Ended);
    deliveries.drain();
    assert_eq!(watch.closed().expect("closed").reason(), "ended");
    assert_eq!(watch.value(), Some(json!(1)));
}

#[test]
fn stops_idempotently_and_rejects_a_repeated_or_late_start() {
    let (watch, _deliveries) = json_watch(json!(0));
    watch.start(Box::new(|_, _, _| Ok(()))).expect("start");
    assert!(watch.start(Box::new(|_, _, _| Ok(()))).is_err());
    assert_eq!(watch.terminate(WatchEnd::Stopped).reason(), "stopped");
    assert_eq!(watch.terminate(WatchEnd::Stopped).reason(), "stopped");

    let (late, _deliveries) = json_watch(json!(0));
    assert_eq!(late.terminate(WatchEnd::Stopped).reason(), "stopped");
    assert!(late.start(Box::new(|_, _, _| Ok(()))).is_err());
}

#[test]
fn ends_ended_from_its_source_dropping_pending_frames() {
    let (watch, deliveries) = json_watch(json!(0));
    let recorded = calls();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    watch.terminate(WatchEnd::Ended);
    deliveries.drain();
    assert!(take(&recorded).is_empty());
    assert_eq!(watch.closed().expect("closed").reason(), "ended");
}

#[test]
fn ends_cancelled_when_its_acquisition_token_is_cancelled() {
    let cancel = CancelToken::new();
    let (watch, deliveries) = json_watch(json!(0));
    watch.observe_cancellation(&cancel);
    cancel.cancel();
    deliveries.drain();
    assert_eq!(watch.closed().expect("closed").reason(), "cancelled");

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let (already, _deliveries) = json_watch(json!(0));
    already.observe_cancellation(&cancelled);
    assert_eq!(already.closed().expect("closed").reason(), "cancelled");
}

#[test]
fn ends_listener_error_on_a_throw_and_delivers_nothing_more() {
    let (watch, deliveries) = json_watch(json!(0));
    let failure: WatchError = Arc::new(std::io::Error::other("listener failed"));
    let thrown = Arc::clone(&failure);
    let values = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&values);
    watch
        .start(Box::new(move |value, _ops, _cancel| {
            recorded
                .lock()
                .expect("values")
                .push(value.expect("value").clone());
            Err(Arc::clone(&thrown))
        }))
        .expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    deliveries.drain();
    watch.advance(json!(2), vec![Op::Replace(json!(2))]);
    deliveries.drain();
    let end = watch.closed().expect("closed");
    assert_eq!(end.reason(), "listener_error");
    assert_eq!(end.error().expect("error").to_string(), "listener failed");
    assert_eq!(*values.lock().expect("values"), vec![json!(1)]);
}

fn native_values(recorded: &Arc<Mutex<Vec<Option<JsonValue>>>>) -> ValueListener<JsonValue> {
    let recorded = Arc::clone(recorded);
    Box::new(move |value, _cancel| {
        recorded.lock().expect("values").push(value.cloned());
        Ok(())
    })
}

fn native(initial: JsonValue) -> (NativeWatch<JsonValue>, Arc<Deliveries>) {
    let deliveries = Arc::new(Deliveries::default());
    let watch = NativeWatch::new(Some(initial), Arc::clone(&deliveries), None);
    (watch, deliveries)
}

#[test]
fn native_watch_delivers_every_value_in_order_never_inline() {
    let (watch, deliveries) = native(json!(0));
    let recorded = Arc::new(Mutex::new(Vec::new()));
    watch.start(native_values(&recorded)).expect("start");
    watch.advance(Some(json!(1)));
    watch.advance(Some(json!(2)));
    assert!(recorded.lock().expect("values").is_empty());
    deliveries.drain();
    assert_eq!(
        *recorded.lock().expect("values"),
        vec![Some(json!(1)), Some(json!(2))]
    );
    assert_eq!(watch.value(), Some(json!(2)));
}

#[test]
fn native_watch_collapses_101_pending_values_to_the_newest_or_its_replace() {
    let (watch, deliveries) = native(json!(0));
    for n in 1..=101 {
        watch.advance(Some(json!(n)));
    }
    let recorded = Arc::new(Mutex::new(Vec::new()));
    watch.start(native_values(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(*recorded.lock().expect("values"), vec![Some(json!(101))]);

    let deliveries = Arc::new(Deliveries::default());
    let replace: Replace<JsonValue> = Arc::new(|| Some(json!("replacement")));
    let replaced = NativeWatch::new(Some(json!(0)), Arc::clone(&deliveries), Some(replace));
    for n in 1..=101 {
        replaced.advance(Some(json!(n)));
    }
    let recorded = Arc::new(Mutex::new(Vec::new()));
    replaced.start(native_values(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(
        *recorded.lock().expect("values"),
        vec![Some(json!("replacement"))]
    );
}

#[test]
fn native_watch_ends_with_every_reason_of_its_lifecycle() {
    let (stopped, _deliveries) = native(json!(0));
    assert_eq!(stopped.terminate(WatchEnd::Stopped).reason(), "stopped");

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let (ended, _deliveries) = native(json!(0));
    ended.observe_cancellation(&cancelled);
    assert_eq!(ended.closed().expect("closed").reason(), "cancelled");

    let (over, _deliveries) = native(json!(0));
    over.terminate(WatchEnd::Ended);
    assert_eq!(over.closed().expect("closed").reason(), "ended");

    let failure: WatchError = Arc::new(std::io::Error::other("listener failed"));
    let thrown = Arc::clone(&failure);
    let (failing, deliveries) = native(json!(0));
    failing
        .start(Box::new(move |_value, _cancel| Err(Arc::clone(&thrown))))
        .expect("start");
    failing.advance(Some(json!(1)));
    deliveries.drain();
    assert_eq!(failing.closed().expect("closed").reason(), "listener_error");
}
