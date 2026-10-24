use pi_chord::delta::{MAX_PENDING_OPS, Op, PendingOps, PendingTake, Seg, apply, track};
use pi_chord::json::JsonValue;
use serde_json::json;

struct Rng(u32);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        f64::from(self.0) / 4_294_967_296.0
    }

    fn int(&mut self, max: usize) -> usize {
        if max == 0 {
            return 0;
        }
        ((self.next() * max as f64) as usize).min(max - 1)
    }

    fn chance(&mut self, probability: f64) -> bool {
        self.next() < probability
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.int(items.len())]
    }

    fn text(&mut self) -> String {
        let texts = ["a", "bc", " tok", "é", "😀", "日本語", "x y", ""];
        self.pick(&texts).to_string()
    }

    fn primitive(&mut self) -> JsonValue {
        match self.int(4) {
            0 => json!(self.int(100)),
            1 => json!(self.text()),
            2 => json!(self.chance(0.5)),
            _ => JsonValue::Null,
        }
    }

    fn value(&mut self, depth: usize) -> JsonValue {
        let roll = self.next();
        if depth > 2 || roll < 0.4 {
            return self.primitive();
        }
        if roll < 0.7 {
            return JsonValue::Array((0..self.int(4)).map(|_| self.value(depth + 1)).collect());
        }
        let mut object = serde_json::Map::new();
        let keys = ["a", "b", "0", "1", "a.b", "x y", "text", "list"];
        for _ in 0..=self.int(4) {
            let key = self.pick(&keys).to_string();
            object.insert(key, self.value(depth + 1));
        }
        JsonValue::Object(object)
    }

    fn root(&mut self) -> JsonValue {
        match self.chance(0.2) {
            true => JsonValue::Array((0..self.int(5)).map(|_| self.value(1)).collect()),
            false => self.value(0),
        }
    }
}

fn paths(value: &JsonValue, prefix: &mut Vec<Seg>, out: &mut Vec<Vec<Seg>>) {
    match value {
        JsonValue::Object(map) => {
            for (key, child) in map {
                prefix.push(Seg::Key(key.clone()));
                out.push(prefix.clone());
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        JsonValue::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                prefix.push(Seg::Index(index));
                out.push(prefix.clone());
                paths(child, prefix, out);
                prefix.pop();
            }
        }
        _ => {}
    }
}

fn at<'a>(value: &'a mut JsonValue, path: &[Seg]) -> &'a mut JsonValue {
    let mut cursor = value;
    for seg in path {
        cursor = match seg {
            Seg::Key(key) => cursor.get_mut(key).expect("path resolves"),
            Seg::Index(index) => cursor.get_mut(*index).expect("path resolves"),
        };
    }
    cursor
}

fn mutate(draft: &mut JsonValue, rnd: &mut Rng) {
    let mut found = Vec::new();
    paths(draft, &mut Vec::new(), &mut found);
    if found.is_empty() {
        *draft = rnd.value(1);
        return;
    }
    let path = found[rnd.int(found.len())].clone();
    let roll = rnd.next();
    let current = at(draft, &path).clone();
    match current {
        JsonValue::String(text) => match roll {
            _ if roll < 0.3 => match at(draft, &path) {
                JsonValue::String(text) => text.push_str(&rnd.text()),
                _ => unreachable!(),
            },
            _ if roll < 0.6 => {
                let keep = text
                    .char_indices()
                    .nth(rnd.int(3))
                    .map(|(at, _)| at)
                    .unwrap_or(text.len());
                *at(draft, &path) = json!(&text[..keep]);
            }
            _ => *at(draft, &path) = rnd.primitive(),
        },
        JsonValue::Array(items) => match roll {
            _ if roll < 0.35 => *at(draft, &path) = json!(items.len()),
            _ if roll < 0.6 => {
                let value = rnd.value(2);
                match at(draft, &path) {
                    JsonValue::Array(items) => items.push(value),
                    _ => unreachable!(),
                }
            }
            _ if roll < 0.75 => {
                let pick = rnd.int(items.len().max(1));
                if let JsonValue::Array(items) = at(draft, &path)
                    && pick < items.len()
                {
                    items.remove(pick);
                }
            }
            _ if roll < 0.9 => {
                let pick = rnd.int(items.len().max(1));
                let value = rnd.value(2);
                if let JsonValue::Array(items) = at(draft, &path)
                    && pick < items.len()
                {
                    items[pick] = value;
                }
            }
            _ => {
                let value = rnd.value(2);
                *at(draft, &path) = value;
            }
        },
        JsonValue::Object(map) => match roll {
            _ if roll < 0.4 => {
                let key = ["a", "b", "1", "x y", "list"][rnd.int(5)].to_string();
                let value = rnd.value(2);
                match at(draft, &path) {
                    JsonValue::Object(map) => map.insert(key, value),
                    _ => None,
                };
            }
            _ if roll < 0.7 && !map.is_empty() => {
                let key = map.keys().nth(rnd.int(map.len())).cloned().expect("key");
                match at(draft, &path) {
                    JsonValue::Object(map) => map.remove(&key),
                    _ => None,
                };
            }
            _ => {
                let value = rnd.value(2);
                *at(draft, &path) = value;
            }
        },
        _ => *at(draft, &path) = rnd.value(2),
    }
}

fn folded_replay(initial: JsonValue, commits: &[(Vec<Op>, JsonValue)]) -> (usize, usize) {
    let mut queue = PendingOps::default();
    let mut replica = initial;
    let mut ops = 0;
    let mut folded = 0;
    for (batch, _value) in commits {
        ops += batch.len();
        queue.push(batch.clone());
    }
    let taken = match queue.take() {
        PendingTake::Ops(ops) => ops,
        PendingTake::Replace => panic!("unexpected overflow"),
    };
    folded += ops - taken.len();
    replica = apply(Some(replica.clone()), &taken).expect("ops apply");
    assert_eq!(replica, commits.last().expect("commits").1);
    (ops, folded)
}

#[test]
fn folding_is_invisible_to_a_replica() {
    let mut rnd = Rng(1);
    let mut all_ops = 0;
    let mut all_folded = 0;
    for _ in 0..200 {
        let initial = rnd.root();
        let mut tracker = track(initial.clone());
        let mut commits = Vec::new();
        for _ in 0..1 + rnd.int(20) {
            let mut change = tracker.begin_change();
            for _ in 0..1 + rnd.int(4) {
                mutate(change.state(), &mut rnd);
            }
            let prepared = change.prepare(&tracker).expect("ops");
            let batch = prepared.ops().to_vec();
            tracker.adopt(prepared).expect("adopt");
            let value = tracker.value().clone();
            commits.push((batch, value));
        }
        let (ops, folded) = folded_replay(initial, &commits);
        all_ops += ops;
        all_folded += folded;
    }
    assert!(
        all_folded * 10 < all_ops * 9,
        "folded {all_folded} of {all_ops} ops"
    );
}

fn fold(ops: Vec<Vec<Op>>) -> Vec<Op> {
    let mut queue = PendingOps::default();
    for batch in ops {
        queue.push(batch);
    }
    match queue.take() {
        PendingTake::Ops(ops) => ops,
        PendingTake::Replace => panic!("unexpected overflow"),
    }
}

fn key(name: &str) -> Seg {
    Seg::Key(name.to_owned())
}

#[test]
fn fold_is_invisible_to_a_replica_across_takes() {
    let initial = json!({ "text": "", "n": 0 });
    let mut queue = PendingOps::default();
    let mut replica = initial.clone();
    let mut value = initial;
    for token in 1..=30 {
        value["text"] = json!(format!("{}tok ", value["text"].as_str().unwrap_or("")));
        value["n"] = json!(token);
        queue.push(vec![
            Op::Append(vec![key("text")], "tok ".into()),
            Op::Set(vec![key("n")], json!(token)),
        ]);
        if token % 7 == 0 {
            let taken = match queue.take() {
                PendingTake::Ops(ops) => ops,
                PendingTake::Replace => panic!("unexpected overflow"),
            };
            replica = apply(Some(replica), &taken).expect("ops apply");
            assert_eq!(replica, value);
        }
    }
    let taken = match queue.take() {
        PendingTake::Ops(ops) => ops,
        PendingTake::Replace => panic!("unexpected overflow"),
    };
    if !taken.is_empty() {
        replica = apply(Some(replica), &taken).expect("ops apply");
    }
    assert_eq!(replica, value);
}

#[test]
fn r_drops_everything_before_it() {
    let folded = fold(vec![
        vec![Op::Set(vec![key("a")], json!(1))],
        vec![Op::Append(vec![key("b")], "x".into())],
        vec![Op::Replace(json!({ "c": 3 }))],
        vec![Op::Set(vec![key("d")], json!(4))],
    ]);
    assert_eq!(
        folded,
        vec![
            Op::Replace(json!({ "c": 3 })),
            Op::Set(vec![key("d")], json!(4)),
        ]
    );
}

#[test]
fn joins_appends_and_folds_appends_into_a_string_set() {
    let folded = fold(vec![
        vec![Op::Set(vec![key("text")], json!("a"))],
        vec![Op::Append(vec![key("text")], "b".into())],
        vec![Op::Append(vec![key("text")], "c".into())],
    ]);
    assert_eq!(folded, vec![Op::Set(vec![key("text")], json!("abc"))]);
    assert_eq!(
        fold(vec![
            vec![Op::Append(vec![key("head")], "a".into())],
            vec![Op::Append(vec![key("head")], "b".into())],
        ]),
        vec![Op::Append(vec![key("head")], "ab".into())]
    );
}

#[test]
fn keys_of_one_parent_never_collide() {
    let folded = fold(vec![vec![
        Op::Set(vec![Seg::Key("a.b".into())], json!(1)),
        Op::Set(vec![key("a"), key("b")], json!(2)),
    ]]);
    assert_eq!(
        folded,
        vec![
            Op::Set(vec![Seg::Key("a.b".into())], json!(1)),
            Op::Set(vec![key("a"), key("b")], json!(2)),
        ]
    );
}

#[test]
fn a_structural_op_clears_the_leaf_index() {
    let folded = fold(vec![
        vec![Op::Append(vec![key("text")], "a".into())],
        vec![Op::Truncate(vec![key("text")], 1)],
        vec![Op::Append(vec![key("text")], "b".into())],
    ]);
    assert_eq!(
        folded,
        vec![
            Op::Append(vec![key("text")], "a".into()),
            Op::Truncate(vec![key("text")], 1),
            Op::Append(vec![key("text")], "b".into()),
        ]
    );
}

#[test]
fn a_set_of_a_container_is_not_a_leaf() {
    let folded = fold(vec![
        vec![Op::Set(vec![key("doc")], json!({ "n": 1 }))],
        vec![Op::Set(vec![key("doc")], json!({ "n": 2 }))],
    ]);
    assert_eq!(folded.len(), 2);
}

#[test]
fn keeps_max_pending_ops_and_replaces_past_them() {
    assert_eq!(fold(self_distinct(MAX_PENDING_OPS)).len(), MAX_PENDING_OPS);

    let mut queue = PendingOps::default();
    for batch in self_distinct(MAX_PENDING_OPS + 1) {
        queue.push(batch);
    }
    assert!(matches!(queue.take(), PendingTake::Replace));
}

fn self_distinct(count: usize) -> Vec<Vec<Op>> {
    (0..count)
        .map(|index| vec![Op::Set(vec![key("k"), Seg::Index(index)], json!(index))])
        .collect()
}

#[test]
fn counts_ops_after_folding_and_nothing_once_taken() {
    let mut queue = PendingOps::default();
    assert_eq!(queue.size(), 0);
    for batch in self_distinct(10) {
        queue.push(batch);
    }
    assert_eq!(queue.size(), 10);
    queue.push(vec![
        Op::Append(vec![key("text")], "a".into()),
        Op::Append(vec![key("text")], "b".into()),
    ]);
    assert_eq!(queue.size(), 11);
    let _ = queue.take();
    assert_eq!(queue.size(), 0);
}

#[test]
fn accumulates_nothing_once_replaced_until_taken_or_a_batch_starting_with_r() {
    let mut queue = PendingOps::default();
    for batch in self_distinct(MAX_PENDING_OPS + 1) {
        queue.push(batch);
    }
    assert_eq!(queue.size(), 1);
    queue.push(vec![Op::Set(vec![key("late")], json!(1))]);
    assert_eq!(queue.size(), 1);
    queue.push(vec![Op::Replace(json!({ "fresh": true }))]);
    assert_eq!(queue.size(), 1);
    let taken = match queue.take() {
        PendingTake::Ops(ops) => ops,
        PendingTake::Replace => panic!("a batch starting with r replaces the overflow"),
    };
    assert_eq!(taken, vec![Op::Replace(json!({ "fresh": true }))]);
}

#[test]
fn hands_one_batch_over_as_received_whatever_its_size() {
    let batch = self_distinct(3);
    let mut queue = PendingOps::default();
    for item in batch {
        queue.push(item);
    }
    let taken = match queue.take() {
        PendingTake::Ops(ops) => ops,
        PendingTake::Replace => panic!("no overflow"),
    };
    assert_eq!(taken.len(), 3);
    assert_eq!(
        taken,
        vec![
            Op::Set(vec![key("k"), Seg::Index(0)], json!(0)),
            Op::Set(vec![key("k"), Seg::Index(1)], json!(1)),
            Op::Set(vec![key("k"), Seg::Index(2)], json!(2)),
        ]
    );
}

#[test]
fn never_mutates_received_ops() {
    let received = vec![Op::Append(vec![key("text")], "a".into())];
    let before = received.clone();
    let mut queue = PendingOps::default();
    queue.push(received.clone());
    queue.push(vec![Op::Append(vec![key("text")], "b".into())]);
    let _ = queue.take();
    assert_eq!(received, before);
}
