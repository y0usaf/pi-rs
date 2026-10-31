//! The --session and --fork argument (main.ts resolveSessionPath), driven through the built
//! binary: a path names a file, a whole id or a prefix names an existing session of this project
//! or of another one, and an id is never written as a file name beside the cwd.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fail<T>(message: String) -> Result<T> {
    Err(message.into())
}

fn executable(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

fn binary() -> Result<PathBuf> {
    let path = match std::env::var("PI_RS_BIN") {
        Ok(value) if !value.is_empty() => PathBuf::from(value),
        _ => std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .map(|dir| Path::new(dir).join("pi-rs"))
            .find(|candidate| executable(candidate))
            .ok_or("pi-rs not found: set PI_RS_BIN to the built binary")?,
    };
    if !executable(&path) {
        return fail(format!(
            "{} is not an executable file (set PI_RS_BIN)",
            path.display()
        ));
    }
    Ok(std::fs::canonicalize(path)?)
}

/// What rpc mode answers to a get_state, and the answer that names the session file.
const GET_STATE: &str = "{\"id\":\"1\",\"type\":\"get_state\"}\n";

const PROMPT: &str = "{\"id\":\"0\",\"type\":\"prompt\",\"message\":\"hi\"}\n{\"id\":\"1\",\"type\":\"get_state\"}\n";

fn reply(stream: TcpStream) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream);
    let mut length = 0;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    reader.read_exact(&mut vec![0; length])?;
    let body = [
        json!({"id": "c", "choices": [{"index": 0, "delta": {"role": "assistant", "content": "ok"}}]}),
        json!({"id": "c", "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
    ]
    .iter()
    .map(|chunk| format!("data: {chunk}\n\n"))
    .collect::<String>()
        + "data: [DONE]\n\n";
    write!(
        reader.get_mut(),
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn serve() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if let Err(error) = reply(stream) {
                eprintln!("fake model: {error}");
            }
        }
    });
    Ok(port)
}

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(port: u16) -> Result<Self> {
        let root = std::env::temp_dir().join(format!("session-e2e-{}", std::process::id()));
        for dir in ["home", "agent", "a", "b"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        let models = json!({"providers": {"fake": {
            "baseUrl": format!("http://127.0.0.1:{port}/v1"),
            "apiKey": "sk-test",
            "api": "openai-completions",
            "models": [{"id": "fake-test", "contextWindow": 200000, "maxTokens": 8192}],
        }}});
        std::fs::write(root.join("agent/models.json"), serde_json::to_vec(&models)?)?;
        Ok(Self { root })
    }

    fn project(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn run(&self, bin: &Path, project: &str, args: &[&str], stdin: &str) -> Result<Output> {
        let mut child = Command::new(bin)
            .args(args)
            .current_dir(self.project(project))
            .env_clear()
            .env("HOME", self.root.join("home"))
            .env("PI_CODING_AGENT_DIR", self.root.join("agent"))
            .env("PATH", "/nonexistent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .ok_or("no stdin")?
            .write_all(stdin.as_bytes())?;
        Ok(child.wait_with_output()?)
    }

    /// Every session file this run's agent directory holds.
    fn files(&self) -> Result<BTreeSet<PathBuf>> {
        let mut found = BTreeSet::new();
        let mut dirs = vec![self.root.join("agent").join("sessions")];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|ext| ext == "db") {
                    found.insert(path);
                }
            }
        }
        Ok(found)
    }

    /// A project's cwd, which a run must leave as empty as it found it.
    fn beside(&self, project: &str) -> Result<Vec<String>> {
        let mut names: Vec<String> = std::fs::read_dir(self.project(project))?
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        Ok(names)
    }
}

fn answered(output: &Output) -> Option<PathBuf> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|line| line["type"] == "response" && line["id"] == "1")
        .and_then(|line| line["data"]["sessionFile"].as_str().map(PathBuf::from))
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn need_files(scratch: &Scratch, expected: &BTreeSet<PathBuf>, what: &str) -> Result<()> {
    let found = scratch.files()?;
    if &found != expected {
        return fail(format!("{what}: the session files are {found:?}"));
    }
    Ok(())
}

fn main() -> Result<()> {
    let bin = binary()?;
    let scratch = Scratch::new(serve()?)?;
    let base = [
        "--provider",
        "fake",
        "--model",
        "fake-test",
        "--mode",
        "rpc",
    ];

    let unstarted = scratch.run(&bin, "a", &base, GET_STATE)?;
    if !unstarted.status.success() || answered(&unstarted).is_none() {
        return fail(format!(
            "a run with no prompt exited {}: {}",
            unstarted.status,
            String::from_utf8_lossy(&unstarted.stderr)
        ));
    }
    need_files(&scratch, &BTreeSet::new(), "a run with no prompt")?;
    println!("a run with no prompt leaves no session file");

    let created = scratch.run(&bin, "a", &base, PROMPT)?;
    if !created.status.success() {
        return fail(format!(
            "pi-rs {} exited {}: {}",
            base.join(" "),
            created.status,
            String::from_utf8_lossy(&created.stderr)
        ));
    }
    let source = answered(&created).ok_or("get_state named no sessionFile")?;
    let id = stem(&source);
    let alone: BTreeSet<PathBuf> = [source.clone()].into();
    need_files(&scratch, &alone, "the first run")?;
    println!("a session in project a is {}", source.display());

    for arg in [id.clone(), id.chars().take(8).collect()] {
        let mut args = base.to_vec();
        args.extend(["--session", &arg]);
        let out = scratch.run(&bin, "a", &args, GET_STATE)?;
        if !out.status.success() {
            return fail(format!("--session {arg} exited {}", out.status));
        }
        if answered(&out).as_deref() != Some(source.as_path()) {
            return fail(format!(
                "--session {arg} opened {:?}, not {}",
                answered(&out),
                source.display()
            ));
        }
        need_files(&scratch, &alone, &format!("--session {arg}"))?;
        println!("--session {arg} resumes it");
    }
    let names = scratch.beside("a")?;
    if !names.is_empty() {
        return fail(format!(
            "project a holds {names:?}: an id was written as a file name"
        ));
    }

    let prefix: String = id.chars().take(8).collect();
    let mut args = base.to_vec();
    args.extend(["--session", &prefix]);
    let project_a = std::fs::canonicalize(scratch.project("a"))?;
    let asked = format!(
        "Session found in different project: {}\nFork this session into current directory? [y/N] ",
        project_a.display()
    );
    let out = scratch.run(&bin, "b", &args, "")?;
    if !out.status.success() || String::from_utf8_lossy(&out.stderr) != asked {
        return fail(format!(
            "an unanswered prompt exited {} with {:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    need_files(&scratch, &alone, "an unanswered prompt")?;
    println!("an unanswered prompt changes nothing");

    let out = scratch.run(&bin, "b", &args, "no\n")?;
    if !out.status.success() || String::from_utf8_lossy(&out.stderr) != format!("{asked}Aborted.\n")
    {
        return fail(format!(
            "a refused fork exited {} with {:?}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    need_files(&scratch, &alone, "a refused fork")?;
    println!("a refused fork changes nothing");

    let out = scratch.run(&bin, "b", &args, &format!("yes\n{GET_STATE}"))?;
    let forked = answered(&out).ok_or("the fork named no sessionFile")?;
    if forked.parent() == source.parent() || !forked.is_file() {
        return fail(format!(
            "the fork is {}, not a new session of project b",
            forked.display()
        ));
    }
    let mut expected = alone.clone();
    expected.insert(forked.clone());
    need_files(&scratch, &expected, "an accepted fork")?;
    let parent = forked.parent().and_then(|dir| {
        pi_coding_agent::session_list::sessions(&dir.to_string_lossy())
            .into_iter()
            .find(|entry| entry.id == stem(&forked))
            .and_then(|entry| entry.parent)
    });
    if parent.as_deref() != Some(source.to_string_lossy().as_ref()) {
        return fail(format!(
            "the fork records {parent:?} as its source, not {}",
            source.display()
        ));
    }
    println!("an accepted fork copies it into project b");

    let mut args = base.to_vec();
    args.extend(["--fork", &prefix]);
    let out = scratch.run(&bin, "b", &args, GET_STATE)?;
    let copied = answered(&out).ok_or("--fork named no sessionFile")?;
    if copied == forked || !copied.is_file() {
        return fail(format!("--fork wrote {}", copied.display()));
    }
    expected.insert(copied);
    need_files(&scratch, &expected, "--fork by prefix")?;
    println!("--fork {prefix} finds project a's session");

    for flag in ["--session", "--fork"] {
        let mut args = base.to_vec();
        args.extend([flag, "no-such-session"]);
        let out = scratch.run(&bin, "a", &args, "")?;
        let stderr = String::from_utf8_lossy(&out.stderr);
        if out.status.code() != Some(1)
            || !stderr.contains("No session found matching 'no-such-session'")
        {
            return fail(format!(
                "{flag} no-such-session exited {} with {stderr:?}",
                out.status
            ));
        }
        need_files(&scratch, &expected, &format!("{flag} with no match"))?;
        println!("{flag} no-such-session exits 1");
    }

    for project in ["a", "b"] {
        let names = scratch.beside(project)?;
        if !names.is_empty() {
            return fail(format!("project {project} holds {names:?}"));
        }
    }
    std::fs::remove_dir_all(&scratch.root)?;
    println!("session e2e ok");
    Ok(())
}
