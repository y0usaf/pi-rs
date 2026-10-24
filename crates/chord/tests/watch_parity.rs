use std::sync::{Arc, Mutex};

use pi_chord::delta::Op;
use pi_chord::json::JsonValue;
use pi_chord::watch::{
    Deliveries, FrameListener, NativeWatch, Overflow, ValueListener, Watch, WatchEnd, WatchError,
};
use pi_rt::cancel::CancelToken;
use serde_json::json;

#[derive(Debug, PartialEq)]
struct Frame {
    value: Option<JsonValue>,
    ops: Vec<Op>,
}

type Frames = Arc<Mutex<Vec<Frame>>>;

fn frames() -> Frames {
    Arc::new(Mutex::new(Vec::new()))
}

fn collector(frames: &Frames) -> FrameListener<JsonValue> {
    let frames = Arc::clone(frames);
    Box::new(move |value, ops, _cancel| {
        frames.lock().expect("frames").push(Frame {
            value: value.cloned(),
            ops: ops.to_vec(),
        });
        Ok(())
    })
}

fn replacing() -> Overflow<JsonValue> {
    Arc::new(|value: &JsonValue| Ok((value.clone(), vec![Op::Replace(value.clone())])))
}

fn json_watch(initial: JsonValue) -> (Watch<JsonValue>, Arc<Deliveries>) {
    let deliveries = Arc::new(Deliveries::default());
    let watch = Watch::new(Some(initial), Arc::clone(&deliveries), replacing());
    (watch, deliveries)
}

fn set(key: &str) -> Vec<Op> {
    vec![Op::Set(
        vec![pi_chord::delta::Seg::Key(key.into())],
        json!(1),
    )]
}

fn snapshot(frames: &Frames) -> Vec<Frame> {
    std::mem::take(&mut *frames.lock().expect("frames"))
}

#[test]
fn queued_frames_deliver_in_order_once_started() {
    let (watch, deliveries) = json_watch(json!({ "n": 0 }));
    watch.advance(json!({ "n": 1 }), set("n"));
    watch.advance(json!({ "n": 2 }), set("n"));
    deliveries.drain();
    assert_eq!(watch.value(), Some(json!({ "n": 0 })));

    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    assert!(snapshot(&recorded).is_empty());
    deliveries.drain();
    assert_eq!(
        snapshot(&recorded),
        vec![
            Frame {
                value: Some(json!({ "n": 1 })),
                ops: set("n"),
            },
            Frame {
                value: Some(json!({ "n": 2 })),
                ops: set("n"),
            },
        ]
    );
    assert_eq!(watch.value(), Some(json!({ "n": 2 })));
}

#[test]
fn never_invokes_the_listener_inline() {
    let (watch, deliveries) = json_watch(json!(0));
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    assert!(snapshot(&recorded).is_empty());
    deliveries.drain();
    assert_eq!(
        snapshot(&recorded),
        vec![Frame {
            value: Some(json!(1)),
            ops: vec![Op::Replace(json!(1))],
        }]
    );
}

#[test]
fn delivers_frames_advanced_during_a_call_after_that_call() {
    let (watch, deliveries) = json_watch(json!(0));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let advanced = watch.clone();
    let recorded = Arc::clone(&calls);
    watch
        .start(Box::new(move |value, _ops, _cancel| {
            let number = value.expect("value");
            recorded.lock().expect("calls").push(number.clone());
            if *number == json!(1) {
                for n in 2..=4 {
                    advanced.advance(json!(n), vec![Op::Replace(json!(n))]);
                }
            }
            Ok(())
        }))
        .expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    deliveries.drain();
    assert_eq!(
        *calls.lock().expect("calls"),
        vec![json!(1), json!(2), json!(3), json!(4)]
    );
}

#[test]
fn collapses_101_pending_frames_to_one_root_replacement() {
    let (watch, deliveries) = json_watch(json!(0));
    for n in 1..=101 {
        watch.advance(json!(n), vec![Op::Replace(json!(n))]);
    }
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(
        snapshot(&recorded),
        vec![Frame {
            value: Some(json!(101)),
            ops: vec![Op::Replace(json!(101))],
        }]
    );
}

#[test]
fn keeps_100_pending_frames_exact() {
    let (watch, deliveries) = json_watch(json!(0));
    for n in 1..=100 {
        watch.advance(json!(n), vec![Op::Replace(json!(n))]);
    }
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    deliveries.drain();
    let values: Vec<JsonValue> = snapshot(&recorded)
        .into_iter()
        .map(|frame| frame.value.expect("value"))
        .collect();
    assert_eq!(values, (1..=100).map(|n| json!(n)).collect::<Vec<_>>());
}

#[test]
fn delivers_the_value_of_the_overflow_policy() {
    let deliveries = Arc::new(Deliveries::default());
    let overflow: Overflow<JsonValue> = Arc::new(|_value: &JsonValue| {
        Ok((
            json!("replacement"),
            vec![Op::Replace(json!("replacement"))],
        ))
    });
    let watch = Watch::new(Some(json!(0)), Arc::clone(&deliveries), overflow);
    for n in 1..=101 {
        watch.advance(json!(n), Vec::new());
    }
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(
        snapshot(&recorded),
        vec![Frame {
            value: Some(json!("replacement")),
            ops: vec![Op::Replace(json!("replacement"))],
        }]
    );
}

#[test]
fn never_folds_the_in_flight_frame_into_an_overflow() {
    let (watch, deliveries) = json_watch(json!(0));
    let values = Arc::new(Mutex::new(Vec::new()));
    let advanced = watch.clone();
    let recorded = Arc::clone(&values);
    watch
        .start(Box::new(move |value, _ops, _cancel| {
            let number = value.expect("value");
            recorded.lock().expect("values").push(number.clone());
            if *number == json!(1) {
                for n in 2..=102 {
                    advanced.advance(json!(n), vec![Op::Replace(json!(n))]);
                }
            }
            Ok(())
        }))
        .expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    deliveries.drain();
    assert_eq!(*values.lock().expect("values"), vec![json!(1), json!(102)]);
}

#[test]
fn delivers_the_retirement_frame_last_and_then_ends_retired() {
    let (watch, deliveries) = json_watch(json!(1));
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(json!(2), vec![Op::Replace(json!(2))]);
    watch.retire();
    deliveries.drain();
    assert_eq!(
        snapshot(&recorded),
        vec![
            Frame {
                value: Some(json!(2)),
                ops: vec![Op::Replace(json!(2))],
            },
            Frame {
                value: None,
                ops: vec![Op::Replace(JsonValue::Null)],
            },
        ]
    );
    assert_eq!(watch.value(), None);
    assert_eq!(watch.closed().expect("closed").reason(), "retired");
}

#[test]
fn drops_an_overflowing_backlog_for_the_retirement_frame() {
    let (watch, deliveries) = json_watch(json!(0));
    for n in 1..=100 {
        watch.advance(json!(n), vec![Op::Replace(json!(n))]);
    }
    watch.retire();
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(
        snapshot(&recorded),
        vec![Frame {
            value: None,
            ops: vec![Op::Replace(JsonValue::Null)],
        }]
    );
    assert_eq!(watch.closed().expect("closed").reason(), "retired");
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
    let recorded = frames();
    watch.start(collector(&recorded)).expect("start");
    watch.advance(json!(1), vec![Op::Replace(json!(1))]);
    watch.terminate(WatchEnd::Ended);
    deliveries.drain();
    assert!(snapshot(&recorded).is_empty());
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

#[test]
fn native_watch_delivers_every_value_in_order_never_inline() {
    let deliveries = Arc::new(Deliveries::default());
    let watch = NativeWatch::new(Some(json!(0)), Arc::clone(&deliveries), replacing());
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
fn native_watch_collapses_101_pending_values_to_the_newest() {
    let deliveries = Arc::new(Deliveries::default());
    let watch = NativeWatch::new(Some(json!(0)), Arc::clone(&deliveries), replacing());
    for n in 1..=101 {
        watch.advance(Some(json!(n)));
    }
    let recorded = Arc::new(Mutex::new(Vec::new()));
    watch.start(native_values(&recorded)).expect("start");
    deliveries.drain();
    assert_eq!(*recorded.lock().expect("values"), vec![Some(json!(101))]);
}

#[test]
fn native_watch_ends_with_every_reason_of_its_lifecycle() {
    let deliveries = Arc::new(Deliveries::default());
    let create = || NativeWatch::new(Some(json!(0)), Arc::clone(&deliveries), replacing());

    let stopped = create();
    assert_eq!(stopped.terminate(WatchEnd::Stopped).reason(), "stopped");

    let cancelled = CancelToken::new();
    cancelled.cancel();
    let ended = create();
    ended.observe_cancellation(&cancelled);
    assert_eq!(ended.closed().expect("closed").reason(), "cancelled");

    let over = create();
    over.terminate(WatchEnd::Ended);
    assert_eq!(over.closed().expect("closed").reason(), "ended");

    let failure: WatchError = Arc::new(std::io::Error::other("source failed"));
    let thrown = Arc::clone(&failure);
    let failing = create();
    failing
        .start(Box::new(move |_value, _cancel| Err(Arc::clone(&thrown))))
        .expect("start");
    failing.advance(Some(json!(1)));
    deliveries.drain();
    assert_eq!(failing.closed().expect("closed").reason(), "listener_error");
}
