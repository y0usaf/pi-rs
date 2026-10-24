use super::op::{Op, Path, Seg, check};
use super::overlap::overlap;
use crate::error::Error;
use crate::json::JsonValue;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OWNER: AtomicU64 = AtomicU64::new(1);

const OVERLAP_SCAN: usize = 65_536;
const OVERLAP_PROBE: usize = 64;
const OVERLAP_CANDIDATES: usize = 8;

pub struct Tracker {
    owner: u64,
    value: JsonValue,
    revision: u64,
}

pub struct Change {
    owner: u64,
    base_revision: u64,
    base: JsonValue,
    draft: JsonValue,
    appends: Vec<(Path, String)>,
}

pub struct Prepared {
    owner: u64,
    base_revision: u64,
    value: JsonValue,
    ops: Vec<Op>,
}

pub fn track(initial: JsonValue) -> Tracker {
    Tracker {
        owner: NEXT_OWNER.fetch_add(1, Ordering::Relaxed),
        value: initial,
        revision: 0,
    }
}

impl Tracker {
    pub fn value(&self) -> &JsonValue {
        &self.value
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn begin_change(&self) -> Change {
        Change {
            owner: self.owner,
            base_revision: self.revision,
            base: self.value.clone(),
            draft: self.value.clone(),
            appends: Vec::new(),
        }
    }

    pub fn prepare_replace(&self, value: JsonValue) -> Result<Prepared, Error> {
        let ops = if equal_trusted(&self.value, &value) {
            Vec::new()
        } else {
            let op = Op::Replace(value.clone());
            check(&op)?;
            vec![op]
        };
        Ok(Prepared {
            owner: self.owner,
            base_revision: self.revision,
            value,
            ops,
        })
    }

    pub fn adopt(&mut self, prepared: Prepared) -> Result<(), Error> {
        if prepared.owner != self.owner {
            return Err(Error::ForeignPrepared);
        }
        if prepared.base_revision != self.revision {
            return Err(Error::Stale);
        }
        self.value = prepared.value;
        self.revision += 1;
        Ok(())
    }
}

impl Change {
    pub fn state(&mut self) -> &mut JsonValue {
        &mut self.draft
    }

    pub fn append_text(&mut self, path: &[Seg], text: &str) -> Result<(), Error> {
        if text.is_empty() {
            return Ok(());
        }
        if path.is_empty() {
            return Err(Error::Path(Vec::new()));
        }
        let target = at(&mut self.draft, path).ok_or_else(|| Error::Path(path.to_vec()))?;
        let JsonValue::String(current) = target else {
            return Err(Error::State(format!(
                "append_text needs a string at {path:?}"
            )));
        };
        current.push_str(text);
        match self.appends.iter_mut().find(|(known, _)| known == path) {
            Some((_, appended)) => appended.push_str(text),
            None => self.appends.push((path.to_vec(), text.to_string())),
        }
        Ok(())
    }

    pub fn prepare(mut self, tracker: &Tracker) -> Result<Prepared, Error> {
        if self.owner != tracker.owner {
            return Err(Error::ForeignPrepared);
        }
        let mut ops = Vec::new();
        diff(
            &mut Vec::new(),
            &self.base,
            &self.draft,
            &mut ops,
            &mut self.appends,
        );
        ops.iter().try_for_each(check)?;
        Ok(Prepared {
            owner: self.owner,
            base_revision: self.base_revision,
            value: self.draft,
            ops,
        })
    }
}

impl Prepared {
    pub fn value(&self) -> &JsonValue {
        &self.value
    }

    pub fn ops(&self) -> &[Op] {
        &self.ops
    }
}

fn equal_trusted(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Array(x), JsonValue::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| equal_trusted(p, q))
        }
        (JsonValue::Object(x), JsonValue::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| equal_trusted(v, w)))
        }
        _ => a == b,
    }
}

fn same(a: &JsonValue, b: &JsonValue) -> bool {
    match (a, b) {
        (JsonValue::Array(x), JsonValue::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q))
        }
        (JsonValue::Object(x), JsonValue::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .zip(y.iter())
                    .all(|((k1, v1), (k2, v2))| k1 == k2 && same(v1, v2))
        }
        _ => a == b,
    }
}

fn leaf(path: &Path, value: &JsonValue, ops: &mut Vec<Op>) {
    if path.is_empty() {
        ops.push(Op::Replace(value.clone()));
    } else {
        ops.push(Op::Set(path.clone(), value.clone()));
    }
}

fn at<'a>(value: &'a mut JsonValue, path: &[Seg]) -> Option<&'a mut JsonValue> {
    let mut cursor = value;
    for seg in path {
        cursor = match seg {
            Seg::Key(key) => cursor.get_mut(key)?,
            Seg::Index(index) => cursor.get_mut(*index)?,
        };
    }
    Some(cursor)
}

fn diff(
    path: &mut Path,
    base: &JsonValue,
    next: &JsonValue,
    ops: &mut Vec<Op>,
    appends: &mut Vec<(Path, String)>,
) {
    if let Some(position) = appends.iter().position(|(known, _)| known == path) {
        let (_, appended) = appends.swap_remove(position);
        if let (JsonValue::String(old), JsonValue::String(new)) = (base, next)
            && new.len() == old.len() + appended.len()
            && new.starts_with(old.as_str())
        {
            ops.push(Op::Append(path.clone(), appended));
            return;
        }
    }
    if same(base, next) {
        return;
    }
    match (base, next) {
        (JsonValue::Object(old), JsonValue::Object(new)) => {
            let retained: Vec<&String> = old.keys().filter(|key| new.contains_key(*key)).collect();
            let added: Vec<&String> = new.keys().filter(|key| !old.contains_key(*key)).collect();
            let expected = retained.iter().chain(added.iter()).copied();
            if !expected.eq(new.keys()) {
                leaf(path, next, ops);
                return;
            }
            for key in old.keys().filter(|key| !new.contains_key(*key)) {
                let mut child = path.clone();
                child.push(Seg::Key(key.clone()));
                ops.push(Op::Delete(child));
            }
            for key in retained {
                path.push(Seg::Key(key.clone()));
                diff(path, &old[key], &new[key], ops, appends);
                path.pop();
            }
            for key in added {
                let mut child = path.clone();
                child.push(Seg::Key(key.clone()));
                ops.push(Op::Set(child, new[key].clone()));
            }
        }
        (JsonValue::Array(old), JsonValue::Array(new)) if old.len() == new.len() => {
            for (index, (before, after)) in old.iter().zip(new).enumerate() {
                path.push(Seg::Index(index));
                diff(path, before, after, ops, appends);
                path.pop();
            }
        }
        (JsonValue::Array(old), JsonValue::Array(new)) => {
            let prefix = old.iter().zip(new).take_while(|(a, b)| same(a, b)).count();
            let limit = old.len().min(new.len()) - prefix;
            let suffix = (0..limit)
                .take_while(|&k| same(&old[old.len() - 1 - k], &new[new.len() - 1 - k]))
                .count();
            ops.push(Op::Splice(
                path.clone(),
                prefix,
                old.len() - prefix - suffix,
                new[prefix..new.len() - suffix].to_vec(),
            ));
        }
        (JsonValue::String(old), JsonValue::String(new))
            if !path.is_empty() && new.starts_with(old.as_str()) =>
        {
            ops.push(Op::Append(path.clone(), new[old.len()..].to_string()));
        }
        (JsonValue::String(old), JsonValue::String(new)) if !path.is_empty() => {
            let shared = overlap(old, new, OVERLAP_SCAN, OVERLAP_PROBE, OVERLAP_CANDIDATES);
            if shared == 0 {
                leaf(path, next, ops);
            } else {
                ops.push(Op::Truncate(
                    path.clone(),
                    old.encode_utf16().count() - shared,
                ));
                let units: Vec<u16> = new.encode_utf16().collect();
                if units.len() > shared {
                    ops.push(Op::Append(
                        path.clone(),
                        String::from_utf16_lossy(&units[shared..]),
                    ));
                }
            }
        }
        _ => leaf(path, next, ops),
    }
}
