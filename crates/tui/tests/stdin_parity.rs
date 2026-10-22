//! Golden parity with upstream `stdin-buffer.ts`: random chunkings of terminal input fed to the
//! TypeScript buffer under a fake clock (`parity/gen-stdin-fixtures.mjs`), replayed here.

use pi_tui::stdin_buffer::{StdinBuffer, StdinEvent};
use serde_json::{Value, json};

#[test]
fn stdin_buffer_matches_upstream() {
    let scripts: Vec<Value> = serde_json::from_str(include_str!("fixtures/stdin.json")).unwrap();
    let mut failures = Vec::new();
    for script in &scripts {
        let mut buf = StdinBuffer::new(50, 10);
        let mut now = 0u64;
        let mut events = Vec::new();
        let record = |evs: Vec<StdinEvent>, at: u64, events: &mut Vec<Value>| {
            for e in evs {
                events.push(match e {
                    StdinEvent::Data(d) => json!(["data", d, at]),
                    StdinEvent::Paste(d) => json!(["paste", d, at]),
                });
            }
        };
        for step in script["steps"].as_array().unwrap() {
            if let Some(feed) = step["feed"].as_str() {
                let evs = buf.process(feed, now);
                record(evs, now, &mut events);
            }
            let until = now + step["wait"].as_u64().unwrap();
            while let Some(d) = buf.deadline().filter(|d| *d <= until) {
                now = d;
                let evs = buf.tick(now);
                record(evs, now, &mut events);
            }
            now = until;
        }
        let got = json!({ "events": events, "rest": buf.buffer() });
        let want = json!({ "events": script["events"], "rest": script["rest"] });
        if got != want {
            failures.push(format!("steps {}\n  got:  {got}\n  want: {want}", script["steps"]));
        }
    }
    let total = failures.len();
    failures.truncate(10);
    assert!(total == 0, "{total} of {} scripts differ:\n{}", scripts.len(), failures.join("\n"));
}

#[test]
fn astral_characters_stay_whole() {
    // Upstream emits the two UTF-16 halves of an astral character as separate events.
    let mut buf = StdinBuffer::default();
    assert_eq!(buf.process("a\u{1F600}", 0), vec![StdinEvent::Data("a".into()), StdinEvent::Data("\u{1F600}".into())]);
}
