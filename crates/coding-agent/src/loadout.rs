use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use indexmap::IndexMap;
use pi_durable::cancel::CancelToken;
use pi_durable::errors::Error;
use pi_durable::harness::registry::Registry;
use pi_durable::harness::types::{
    Extension, HookRegistration, OutputLimitsSpec, PromptSection, Render, ToolExecute,
    ToolExecutionResult, ToolExposure, ToolRegistration,
};
use serde::Serialize;
use serde_json::Value;

use crate::codemode;
use crate::system_prompt::{PromptInput, build_sections, build_system_prompt};
use crate::tool_search;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Source {
    Builtin,
    Bundled { path: String },
    Extension { path: String },
    Mcp,
}

#[derive(Clone)]
pub struct Entry {
    pub registration: ToolRegistration,
    pub snippet: Option<String>,
    pub guidelines: Vec<String>,
    pub default_active: bool,
    pub source: Source,
}

impl Entry {
    pub fn new(registration: ToolRegistration, source: Source) -> Self {
        Entry {
            registration,
            snippet: None,
            guidelines: Vec::new(),
            default_active: true,
            source,
        }
    }

    pub fn with_prompt(mut self, snippet: Option<&str>, guidelines: &[String]) -> Self {
        self.snippet = normalize_snippet(snippet);
        self.guidelines = normalize_guidelines(guidelines);
        self
    }

    fn name(&self) -> &str {
        &self.registration.tool.name
    }

    fn exposure(&self) -> ToolExposure {
        self.registration.exposure
    }

    fn declarable(&self) -> bool {
        matches!(
            self.exposure(),
            ToolExposure::Direct | ToolExposure::ModelOnly
        )
    }

    fn activated_on_registration(&self) -> bool {
        self.declarable() && self.default_active
    }
}

fn normalize_snippet(text: Option<&str>) -> Option<String> {
    let one_line = text?.split_whitespace().collect::<Vec<_>>().join(" ");
    (!one_line.is_empty()).then_some(one_line)
}

fn normalize_guidelines(guidelines: &[String]) -> Vec<String> {
    let mut unique: Vec<String> = Vec::new();
    for guideline in guidelines.iter().map(|g| g.trim()) {
        if !guideline.is_empty() && !unique.iter().any(|u| u == guideline) {
            unique.push(guideline.to_owned());
        }
    }
    unique
}

#[derive(Clone, Debug, Default)]
pub struct ToolSelection {
    pub active: Vec<String>,
    pub allowed: Option<Vec<String>>,
    pub excluded: Vec<String>,
}

fn is_mcp(name: &str) -> bool {
    name.starts_with("mcp__")
        || [
            crate::mcp::config::LIST_MCP_RESOURCES_TOOL,
            crate::mcp::config::LIST_MCP_RESOURCE_TEMPLATES_TOOL,
            crate::mcp::config::READ_MCP_RESOURCE_TOOL,
        ]
        .contains(&name)
}

impl ToolSelection {
    fn names(&self, name: &str) -> bool {
        self.allowed
            .as_ref()
            .is_none_or(|allowed| crate::run_config::tool_matches(allowed, name))
    }

    fn filters_mcp(&self) -> bool {
        self.allowed.as_ref().is_some_and(|allowed| {
            allowed.is_empty() || allowed.iter().any(|entry| entry.starts_with("mcp__"))
        })
    }

    fn allows(&self, name: &str) -> bool {
        if crate::run_config::tool_matches(&self.excluded, name) {
            return false;
        }
        self.names(name) || (!self.filters_mcp() && is_mcp(name))
    }

    fn activatable(&self, entry: &Entry, searchable: bool) -> bool {
        self.names(entry.name())
            || !is_mcp(entry.name())
            || (entry.exposure() != ToolExposure::Direct && searchable)
    }
}

#[derive(Clone)]
struct Run {
    options: PromptInput,
    forced: Option<String>,
    selected: Vec<String>,
}

struct State {
    extension: Vec<Entry>,
    active: Vec<String>,
    activatable: HashSet<String>,
    base_prompt: Option<PromptInput>,
    run: Option<Run>,
}

pub struct Loadout {
    me: Weak<Loadout>,
    catalog: Vec<Entry>,
    mcp: Vec<Entry>,
    selection: ToolSelection,
    mode: codemode::Mode,
    hooks: Vec<HookRegistration>,
    state: Mutex<State>,
    registries: Mutex<Vec<Weak<Registry>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolView<'a> {
    name: &'a str,
    description: &'a str,
    parameters: &'a Value,
    prompt_guidelines: Option<&'a [String]>,
    exposure: ToolExposure,
    source: &'a Source,
}

#[derive(Serialize)]
pub struct ToolsView<'a> {
    active: &'a [String],
    all: Vec<ToolView<'a>>,
    known: Vec<&'a str>,
}

#[derive(Serialize)]
pub struct PromptView {
    text: String,
    options: PromptInput,
}

pub struct Found {
    pub name: String,
    pub description: String,
}

fn fixed_section(key: &str, text: String) -> PromptSection {
    let render: Render = Arc::new(
        move |_: &pi_durable::harness::types::PromptInput<'_>, _: &CancelToken| {
            Ok(Some(text.clone()))
        },
    );
    PromptSection {
        key: key.to_owned(),
        render,
        tag: Some(false),
    }
}

fn search_execute(
    loadout: impl Fn() -> Option<Arc<Loadout>> + Send + Sync + 'static,
) -> ToolExecute {
    Arc::new(move |args, _api, _cancel| {
        let failed = |message: String| ToolExecutionResult {
            content: Some(vec![crate::tools::text_block(message)]),
            is_error: Some(true),
            details: Some(Value::Object(Default::default())),
            ..ToolExecutionResult::default()
        };
        let args: tool_search::Args = match serde_json::from_value(args) {
            Ok(args) => args,
            Err(error) => return Ok(failed(error.to_string())),
        };
        let (query, limit) = match args.query().and_then(|query| Ok((query, args.limit()?))) {
            Ok(parsed) => parsed,
            Err(message) => return Ok(failed(message)),
        };
        let found = match loadout() {
            Some(loadout) => loadout.search_and_load(query, limit)?,
            None => Vec::new(),
        };
        let loaded: Vec<(String, String)> = found
            .into_iter()
            .map(|found| (found.name, found.description))
            .collect();
        let (text, details) = tool_search::result(&loaded);
        Ok(ToolExecutionResult {
            content: Some(vec![crate::tools::text_block(text)]),
            details: Some(details),
            ..ToolExecutionResult::default()
        })
    })
}

fn tool_search_registration(loadout: Weak<Loadout>) -> ToolRegistration {
    let execute = search_execute(move || loadout.upgrade());
    ToolRegistration {
        tool: pi_durable::ai::types::Tool {
            name: tool_search::TOOL_NAME.to_owned(),
            description: tool_search::description(),
            parameters: tool_search::parameters(),
            constrained_sampling: None,
        },
        replay: None,
        execution_mode: None,
        output_limits: Some(OutputLimitsSpec {
            max_bytes: Some(usize::MAX),
            max_lines: Some(usize::MAX),
            retain: None,
        }),
        prepare_arguments: None,
        exposure: ToolExposure::ModelOnly,
        hidden: false,
        execute,
    }
}

pub struct Parts {
    pub builtins: Vec<Entry>,
    pub mcp: Vec<ToolRegistration>,
    pub extension: Vec<Entry>,
    pub selection: ToolSelection,
    pub mode: codemode::Mode,
    pub hooks: Vec<HookRegistration>,
    pub prompt: Option<PromptInput>,
}

impl Loadout {
    pub fn new(parts: Parts) -> Arc<Loadout> {
        Arc::new_cyclic(|weak: &Weak<Loadout>| {
            let codemode_contribution = codemode::prompt_contribution();
            let codemode_guidelines: Vec<String> = codemode_contribution
                .guidelines
                .iter()
                .map(|g| g.to_string())
                .collect();
            let mut bundled_codemode = Entry::new(
                codemode::registration(codemode::description()),
                Source::Bundled {
                    path: "builtin:codemode".to_owned(),
                },
            )
            .with_prompt(Some(codemode_contribution.snippet), &codemode_guidelines);
            bundled_codemode.default_active = false;
            let mut bundled_search = Entry::new(
                tool_search_registration(weak.clone()),
                Self::search_source(),
            )
            .with_prompt(Some(tool_search::SNIPPET), &[]);
            bundled_search.default_active = false;
            let mut catalog = parts.builtins;
            catalog.extend([bundled_codemode, bundled_search]);
            let loadout = Loadout {
                me: weak.clone(),
                catalog,
                mcp: parts
                    .mcp
                    .into_iter()
                    .map(|registration| Entry::new(registration, Source::Mcp))
                    .collect(),
                selection: parts.selection,
                mode: parts.mode,
                hooks: parts.hooks,
                state: Mutex::new(State {
                    extension: parts.extension,
                    active: Vec::new(),
                    activatable: HashSet::new(),
                    base_prompt: parts.prompt,
                    run: None,
                }),
                registries: Mutex::new(Vec::new()),
            };
            {
                let mut state = lock(&loadout.state);
                loadout.refresh(&mut state, true);
                loadout.rebuild_base_prompt(&mut state);
            }
            loadout
        })
    }

    fn search_source() -> Source {
        Source::Bundled {
            path: "builtin:tool-search".to_owned(),
        }
    }

    fn registered<'a>(&'a self, state: &'a State) -> Vec<&'a Entry> {
        let mut out: Vec<&Entry> = Vec::new();
        for builtin in self
            .catalog
            .iter()
            .filter(|entry| entry.source == Source::Builtin)
        {
            let replaced = state
                .extension
                .iter()
                .find(|entry| entry.name() == builtin.name());
            out.push(replaced.unwrap_or(builtin));
        }
        let bundled = self
            .catalog
            .iter()
            .filter(|entry| entry.source != Source::Builtin);
        for entry in state.extension.iter().chain(bundled).chain(&self.mcp) {
            if !out.iter().any(|known| known.name() == entry.name()) {
                out.push(entry);
            }
        }
        out.retain(|entry| self.selection.allows(entry.name()));
        out
    }

    fn refresh(&self, state: &mut State, startup: bool) {
        let registered = self.registered(state);
        let mut next: Vec<String> = if startup {
            self.selection.active.clone()
        } else {
            state.active.clone()
        };
        next.retain(|name| self.selection.allows(name));
        if let Some(allowed) = &self.selection.allowed {
            next.extend(
                registered
                    .iter()
                    .filter(|entry| {
                        crate::run_config::tool_matches(allowed, entry.name()) && entry.declarable()
                    })
                    .map(|entry| entry.name().to_owned()),
            );
        } else if startup {
            let bundled = self
                .catalog
                .iter()
                .filter(|entry| entry.source != Source::Builtin);
            let custom = state.extension.iter().chain(bundled).chain(&self.mcp);
            next.extend(
                custom
                    .filter(|entry| registered.iter().any(|r| std::ptr::eq(*r, *entry)))
                    .filter(|entry| entry.activated_on_registration())
                    .map(|entry| entry.name().to_owned()),
            );
        } else {
            next.extend(
                registered
                    .iter()
                    .filter(|entry| {
                        entry.activated_on_registration()
                            && !state.activatable.contains(entry.name())
                    })
                    .map(|entry| entry.name().to_owned()),
            );
        }
        let activatable: HashSet<String> = registered
            .iter()
            .filter(|entry| entry.activated_on_registration())
            .map(|entry| entry.name().to_owned())
            .collect();
        state.activatable = activatable;
        self.apply(state, next);
    }

    fn apply(&self, state: &mut State, names: Vec<String>) {
        let registered = self.registered(state);
        let searchable = registered
            .iter()
            .any(|entry| entry.name() == tool_search::TOOL_NAME);
        let mut active: Vec<String> = Vec::new();
        for name in names {
            if active.contains(&name) {
                continue;
            }
            let Some(entry) = registered.iter().find(|entry| entry.name() == name) else {
                continue;
            };
            if entry.exposure() != ToolExposure::Hidden
                && self.selection.activatable(entry, searchable)
            {
                active.push(name);
            }
        }
        state.active = active;
    }

    fn rebuild_base_prompt(&self, state: &mut State) {
        let declared = Self::declared(&self.tools(state));
        if let Some(base) = &mut state.base_prompt {
            base.selected_tools = declared;
        }
    }

    fn tools(&self, state: &State) -> Vec<ToolRegistration> {
        let registered = self.registered(state);
        let mut tools: Vec<ToolRegistration> = state
            .active
            .iter()
            .filter_map(|name| registered.iter().find(|entry| entry.name() == name))
            .map(|entry| {
                let mut registration = entry.registration.clone();
                registration.hidden = false;
                if entry.source == Self::search_source() {
                    let held = self.me.upgrade();
                    registration.execute = search_execute(move || held.clone());
                }
                registration
            })
            .collect();
        tools.extend(
            registered
                .iter()
                .filter(|entry| {
                    !state.active.iter().any(|name| name == entry.name())
                        && matches!(
                            entry.exposure(),
                            ToolExposure::Codemode | ToolExposure::Deferred
                        )
                })
                .map(|entry| {
                    let mut registration = entry.registration.clone();
                    registration.hidden = true;
                    registration
                }),
        );
        codemode::apply_loadout(&mut tools, self.mode);
        tools
    }

    fn base(&self, state: &State) -> Option<PromptInput> {
        let mut input = state.base_prompt.clone()?;
        let mut snippets = IndexMap::new();
        let mut guidelines = IndexMap::new();
        for entry in self.registered(state) {
            if let Some(snippet) = &entry.snippet {
                snippets.insert(entry.name().to_owned(), snippet.clone());
            }
            if !entry.guidelines.is_empty() {
                guidelines.insert(entry.name().to_owned(), entry.guidelines.clone());
            }
        }
        input.tool_snippets = snippets;
        input.tool_guidelines = guidelines;
        Some(input)
    }

    fn effective(&self, base: &PromptInput, run: &Run, declared: Vec<String>) -> PromptInput {
        let mut options = run.options.clone();
        options.selected_tools = declared;
        let mut snippets = base.tool_snippets.clone();
        snippets.extend(run.options.tool_snippets.clone());
        options.tool_snippets = snippets;
        let mut guidelines = base.tool_guidelines.clone();
        guidelines.extend(run.options.tool_guidelines.clone());
        options.tool_guidelines = guidelines;
        options.package_dir = base.package_dir.clone();
        options
    }

    fn declared(tools: &[ToolRegistration]) -> Vec<String> {
        tools
            .iter()
            .filter(|tool| !tool.hidden)
            .map(|tool| tool.tool.name.clone())
            .collect()
    }

    pub fn prompt(&self) -> Option<PromptInput> {
        self.base(&lock(&self.state))
    }

    fn extension(&self, state: &State) -> Extension {
        let tools = self.tools(state);
        let sections = match (self.base(state), &state.run) {
            (None, _) => Vec::new(),
            (Some(mut base), None) => {
                base.selected_tools = Self::declared(&tools);
                build_sections(&base)
            }
            (Some(base), Some(run)) => {
                build_sections(&self.effective(&base, run, Self::declared(&tools)))
            }
        };
        Extension {
            name: "pi-coding-agent".to_owned(),
            tools,
            sections: sections
                .into_iter()
                .map(|(key, text)| fixed_section(&key, text))
                .collect(),
            wraps: Vec::new(),
            hooks: self.hooks.clone(),
            tasks: Vec::new(),
        }
    }

    fn show(&self, state: &State) {
        let Some(live) = crate::extensions::live() else {
            return;
        };
        let Some(base) = self.base(state) else {
            return;
        };
        let text = match &state.run {
            Some(Run {
                forced: Some(forced),
                ..
            }) => forced.clone(),
            Some(run) => build_system_prompt(&self.effective(&base, run, run.selected.clone())),
            None => build_system_prompt(&base),
        };
        let registered = self.registered(state);
        let all = registered
            .iter()
            .map(|entry| ToolView {
                name: entry.name(),
                description: &entry.registration.tool.description,
                parameters: &entry.registration.tool.parameters,
                prompt_guidelines: (!entry.guidelines.is_empty()).then_some(&entry.guidelines),
                exposure: entry.exposure(),
                source: &entry.source,
            })
            .collect();
        let known = state.extension.iter().map(Entry::name).collect();
        live.show_tools(&ToolsView {
            active: &state.active,
            all,
            known,
        });
        live.show_prompt(&PromptView {
            text,
            options: base,
        });
    }

    fn install(&self) -> Result<(), Error> {
        let extension = {
            let state = lock(&self.state);
            self.show(&state);
            self.extension(&state)
        };
        let registries: Vec<Arc<Registry>> = {
            let mut registries = lock(&self.registries);
            registries.retain(|registry| registry.strong_count() > 0);
            registries.iter().filter_map(Weak::upgrade).collect()
        };
        let mut outcome = Ok(());
        for registry in registries {
            if let Err(error) = registry.install(extension.clone()) {
                outcome = Err(error);
            }
        }
        outcome
    }

    pub fn attach(&self, registry: &Arc<Registry>) -> Result<(), Error> {
        lock(&self.registries).push(Arc::downgrade(registry));
        self.install()
    }

    pub fn set_extension_tools(&self, tools: Vec<Entry>) -> Result<(), Error> {
        {
            let mut state = lock(&self.state);
            state.extension = tools;
            self.refresh(&mut state, false);
            self.rebuild_base_prompt(&mut state);
        }
        self.install()
    }

    pub fn set_active(&self, names: Vec<String>) -> Result<(), Error> {
        {
            let mut state = lock(&self.state);
            self.apply(&mut state, names);
            self.rebuild_base_prompt(&mut state);
        }
        self.install()
    }

    pub fn set_skills(&self, skills: Vec<crate::skills::Skill>) -> Result<(), Error> {
        {
            let mut state = lock(&self.state);
            let Some(prompt) = state.base_prompt.as_mut() else {
                return Ok(());
            };
            prompt.skills = skills;
            self.rebuild_base_prompt(&mut state);
        }
        self.install()
    }

    pub fn start_run(
        &self,
        options: PromptInput,
        forced: Option<String>,
        selected_before: &[String],
    ) -> Result<(), Error> {
        {
            let mut state = lock(&self.state);
            if options.selected_tools != selected_before {
                self.apply(&mut state, options.selected_tools.clone());
                self.rebuild_base_prompt(&mut state);
            }
            let selected = Self::declared(&self.tools(&state));
            state.run = Some(Run {
                options,
                forced,
                selected,
            });
        }
        self.install()
    }

    pub fn next_turn(&self) {
        let mut state = lock(&self.state);
        let declared = Self::declared(&self.tools(&state));
        let Some(run) = state.run.as_mut() else {
            return;
        };
        run.selected = declared;
        self.show(&state);
    }

    pub fn end_run(&self) -> Result<(), Error> {
        if lock(&self.state).run.take().is_none() {
            return Ok(());
        }
        self.install()
    }

    pub fn search_and_load(&self, query: &str, limit: usize) -> Result<Vec<Found>, Error> {
        let found = {
            let mut state = lock(&self.state);
            let registered = self.registered(&state);
            let candidates: Vec<&Entry> = registered
                .into_iter()
                .filter(|entry| {
                    matches!(
                        entry.exposure(),
                        ToolExposure::Codemode | ToolExposure::Deferred
                    ) && !state.active.iter().any(|name| name == entry.name())
                })
                .collect();
            let documents: Vec<String> = candidates
                .iter()
                .map(|entry| {
                    tool_search::document(&tool_search::Candidate {
                        name: entry.name(),
                        description: &entry.registration.tool.description,
                        parameters: &entry.registration.tool.parameters,
                    })
                })
                .collect();
            let found: Vec<Found> = tool_search::rank(query, &documents, limit)
                .into_iter()
                .map(|index| Found {
                    name: candidates[index].name().to_owned(),
                    description: candidates[index].registration.tool.description.clone(),
                })
                .collect();
            if found.is_empty() {
                return Ok(found);
            }
            let mut next = state.active.clone();
            next.extend(found.iter().map(|found| found.name.clone()));
            self.apply(&mut state, next);
            self.rebuild_base_prompt(&mut state);
            found
        };
        self.install()?;
        Ok(found)
    }
}
