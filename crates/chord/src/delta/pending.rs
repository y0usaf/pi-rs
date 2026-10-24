use std::collections::HashMap;

use super::{Op, Path};
use crate::json::JsonValue;

pub const MAX_PENDING_OPS: usize = 10_000;

pub enum PendingTake {
    Ops(Vec<Op>),
    Replace,
}

#[derive(Default)]
pub struct PendingOps {
    state: State,
}

#[derive(Default)]
enum State {
    #[default]
    Empty,
    Batch(Vec<Op>),
    Folding(Folding),
    Replace,
}

#[derive(Default)]
struct Folding {
    ops: Vec<Op>,
    leaves: HashMap<String, usize>,
}

impl PendingOps {
    pub fn push(&mut self, ops: Vec<Op>) {
        if ops.is_empty() {
            return;
        }
        if matches!(ops.first(), Some(Op::Replace(_))) {
            self.state = State::Batch(ops);
            return;
        }
        match std::mem::take(&mut self.state) {
            State::Empty => self.state = State::Batch(ops),
            State::Batch(batch) => {
                let mut folding = Folding::default();
                self.state = match folding.fold(batch) && folding.fold(ops) {
                    true => State::Folding(folding),
                    false => State::Replace,
                };
            }
            State::Folding(mut folding) => {
                self.state = match folding.fold(ops) {
                    true => State::Folding(folding),
                    false => State::Replace,
                };
            }
            State::Replace => self.state = State::Replace,
        }
    }

    pub fn size(&self) -> usize {
        match &self.state {
            State::Empty => 0,
            State::Replace => 1,
            State::Batch(ops) => ops.len(),
            State::Folding(folding) => folding.ops.len(),
        }
    }

    pub fn take(&mut self) -> PendingTake {
        match std::mem::take(&mut self.state) {
            State::Empty => PendingTake::Ops(Vec::new()),
            State::Batch(ops) => PendingTake::Ops(ops),
            State::Folding(folding) => PendingTake::Ops(folding.ops),
            State::Replace => PendingTake::Replace,
        }
    }
}

impl Folding {
    fn fold(&mut self, ops: Vec<Op>) -> bool {
        for op in ops {
            match &op {
                Op::Replace(_) => {
                    self.ops.clear();
                    self.leaves.clear();
                }
                Op::Append(..) => {
                    if self.merge_leaf(&op) {
                        continue;
                    }
                }
                Op::Set(_, value) => match is_primitive(value) {
                    true => {
                        if self.merge_leaf(&op) {
                            continue;
                        }
                    }
                    false => self.leaves.clear(),
                },
                _ => self.leaves.clear(),
            }
            self.ops.push(op);
            if self.ops.len() > MAX_PENDING_OPS {
                return false;
            }
        }
        true
    }

    fn merge_leaf(&mut self, op: &Op) -> bool {
        let Some(path) = leaf_path(op) else {
            return false;
        };
        let key = path_key(path);
        if let Some(at) = self.leaves.get(&key).copied()
            && let Some(previous) = self.ops.get_mut(at)
            && let Some(merged) = merge(previous, op)
        {
            *previous = merged;
            return true;
        }
        self.leaves.insert(key, self.ops.len());
        false
    }
}

fn is_primitive(value: &JsonValue) -> bool {
    !matches!(value, JsonValue::Array(_) | JsonValue::Object(_))
}

fn leaf_path(op: &Op) -> Option<&Path> {
    match op {
        Op::Append(path, _) | Op::Set(path, _) => Some(path),
        _ => None,
    }
}

fn path_key(path: &Path) -> String {
    format!("{path:?}")
}

fn merge(previous: &Op, next: &Op) -> Option<Op> {
    let next_text = match next {
        Op::Set(..) => return Some(next.clone()),
        Op::Append(_, text) => text,
        _ => return None,
    };
    match previous {
        Op::Append(path, text) => Some(Op::Append(path.clone(), format!("{text}{next_text}"))),
        Op::Set(path, JsonValue::String(text)) => Some(Op::Set(
            path.clone(),
            JsonValue::String(format!("{text}{next_text}")),
        )),
        _ => None,
    }
}
