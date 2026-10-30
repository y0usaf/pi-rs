use serde_json::{Value, json};

use crate::config::CONFIG_DIR_NAME;
use crate::paths::{dirname, home_dir, join, resolve, within};
use crate::prompt_templates::PromptTemplate;
use crate::skills::Skill;

fn info(path: &str, source: &str, scope: &str, base_dir: &str) -> Value {
    json!({
        "path": path,
        "source": source,
        "scope": scope,
        "origin": "top-level",
        "baseDir": base_dir,
    })
}

fn fallback(path: &str, cwd: &str, agent_dir: &str) -> Value {
    let project = join(cwd, CONFIG_DIR_NAME);
    for (base, scope) in [(agent_dir, "user"), (project.as_str(), "project")] {
        for kind in ["skills", "prompts", "themes", "extensions"] {
            let root = join(base, kind);
            if within(path, &root) {
                return info(path, "local", scope, &root);
            }
        }
    }
    info(path, "local", "temporary", &dirname(path))
}

fn prompt(path: &str, cwd: &str, agent_dir: &str, trusted: bool) -> Value {
    let project = join(cwd, CONFIG_DIR_NAME);
    if trusted && within(path, &join(&project, "prompts")) {
        return info(path, "auto", "project", &project);
    }
    if within(path, &join(agent_dir, "prompts")) {
        return info(path, "auto", "user", agent_dir);
    }
    fallback(path, cwd, agent_dir)
}

fn skill(path: &str, cwd: &str, agent_dir: &str, trusted: bool) -> Value {
    let project = join(cwd, CONFIG_DIR_NAME);
    let user_agents = join(&home_dir(), ".agents");
    let mut auto: Vec<(String, &str, String)> = Vec::new();
    if trusted {
        auto.push((join(&project, "skills"), "project", project.clone()));
        for dir in crate::skills::ancestor_agents_skill_dirs(cwd) {
            if dir != join(&user_agents, "skills") {
                let base = dirname(&dir);
                auto.push((dir, "project", base));
            }
        }
    }
    auto.push((join(agent_dir, "skills"), "user", agent_dir.to_owned()));
    auto.push((join(&user_agents, "skills"), "user", user_agents));
    auto.iter()
        .find(|(dir, _, _)| within(path, dir))
        .map(|(_, scope, base)| info(path, "auto", scope, base))
        .unwrap_or_else(|| fallback(path, cwd, agent_dir))
}

pub fn slash_commands(
    templates: &[PromptTemplate],
    skills: &[Skill],
    cwd: &str,
    agent_dir: &str,
    trusted: bool,
) -> Vec<Value> {
    let (cwd, agent_dir) = (resolve("/", cwd), resolve("/", agent_dir));
    let prompts = templates.iter().map(|template| {
        json!({
            "name": template.name,
            "description": template.description,
            "source": "prompt",
            "sourceInfo": prompt(&template.file_path, &cwd, &agent_dir, trusted),
        })
    });
    let skills = skills.iter().map(|found| {
        json!({
            "name": format!("skill:{}", found.name),
            "description": found.description,
            "source": "skill",
            "sourceInfo": skill(&found.file_path, &cwd, &agent_dir, trusted),
        })
    });
    prompts.chain(skills).collect()
}
