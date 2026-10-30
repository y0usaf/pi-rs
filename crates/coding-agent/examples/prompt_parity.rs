//! Builds the system prompt for each case of `parity/prompt.json`, printing one JSON line per
//! case, for `parity/run.sh` to compare with upstream (`parity/prompt-driver.ts`).
//!
//! Usage: `prompt_parity CASES.json WORKDIR`

use pi_coding_agent::loadout::ToolSelection;
use pi_coding_agent::print::{ToolSettings, extension};
use pi_coding_agent::system_prompt::{AssembleOptions, assemble, build_system_prompt};
use serde_json::{Value, json};
use std::path::Path;

fn strings(v: &Value) -> Option<Vec<String>> {
    v.as_array()
        .map(|a| a.iter().map(|s| s.as_str().unwrap().to_string()).collect())
}

fn main() {
    let mut argv = std::env::args().skip(1);
    let cases_path = argv
        .next()
        .expect("usage: prompt_parity CASES.json WORKDIR");
    let root = argv
        .next()
        .expect("usage: prompt_parity CASES.json WORKDIR");
    let cases: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(cases_path).unwrap()).unwrap();
    for c in cases {
        let base = format!("{root}/{}", c["name"].as_str().unwrap());
        if let Some(files) = c["files"].as_object() {
            for (name, text) in files {
                let path = Path::new(&base).join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text.as_str().unwrap()).unwrap();
            }
        }
        for d in strings(&c["dirs"]).unwrap_or_default() {
            std::fs::create_dir_all(Path::new(&base).join(d)).unwrap();
        }
        if let Some(links) = c["links"].as_object() {
            for (link, target) in links {
                let path = Path::new(&base).join(link);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::os::unix::fs::symlink(target.as_str().unwrap(), path).unwrap();
            }
        }
        let cwd = format!("{base}/{}", c["cwd"].as_str().unwrap_or("home/proj"));
        let agent_dir = format!("{base}/home/.pi/agent");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        // SAFETY: single-threaded driver.
        unsafe {
            std::env::set_var("HOME", format!("{base}/home"));
            std::env::set_var("PI_PACKAGE_DIR", "/pkg");
        }
        let tools = strings(&c["tools"])
            .unwrap_or_else(|| ["read", "bash", "edit", "write"].map(String::from).to_vec());
        let append = strings(&c["appendSystemPrompt"]);
        let skills = strings(&c["skills"]).unwrap_or_default();
        let assembled = assemble(&AssembleOptions {
            cwd: &cwd,
            agent_dir: &agent_dir,
            project_trusted: c["trusted"].as_bool().unwrap_or(true),
            system_prompt: c["systemPrompt"].as_str(),
            append_system_prompt: append.as_deref(),
            no_context_files: c["noContextFiles"].as_bool().unwrap_or(false),
            no_skills: c["noSkills"].as_bool().unwrap_or(false),
            skill_paths: &skills,
        });
        let settings = ToolSettings {
            cwd: cwd.clone(),
            shell_path: None,
            command_prefix: None,
            env: Vec::new(),
            model_takes_images: true,
            codemode_mode: pi_coding_agent::codemode::Mode::On.name().to_owned(),
            codemode_inline_budget: pi_coding_agent::codemode::default_inline_budget(),
        };
        let selection = ToolSelection {
            active: tools,
            ..ToolSelection::default()
        };
        let input = extension(&selection, Some(assembled.input.clone()), settings)
            .prompt()
            .expect("a loadout given a prompt has one");
        let prompt = build_system_prompt(&input).replace(&base, "<base>");
        let diagnostics: Vec<Value> = assembled
            .skill_diagnostics
            .iter()
            .map(|d| {
                json!([
                    d.kind,
                    d.message.replace(&base, "<base>"),
                    d.path.replace(&base, "<base>")
                ])
            })
            .collect();
        println!(
            "{}",
            json!({"name": c["name"], "prompt": prompt, "diagnostics": diagnostics})
        );
    }
}
