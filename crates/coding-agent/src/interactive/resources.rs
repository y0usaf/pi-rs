use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use pi_tui::components::spacer::Spacer;
use pi_tui::tui::{Container, component};

use super::header::ExpandableText;
use super::theme::fg;
use crate::extensions::LoadedExtension;

fn locale_order(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| b.cmp(a))
}

fn display_path(path: &str) -> String {
    let home = crate::paths::home_dir();
    match path.strip_prefix(home.as_str()) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => path.to_owned(),
    }
}

fn extension_display_path(path: &str) -> String {
    let shown = display_path(path);
    for suffix in ["/index.ts", "/index.js"] {
        if let Some(stripped) = shown.strip_suffix(suffix) {
            return stripped.to_owned();
        }
    }
    shown
}

fn scope_group(extension: &LoadedExtension) -> &'static str {
    let info = extension.source_info.as_ref();
    let source = info.map_or("local", |info| info.source.as_str());
    let scope = info.map_or("project", |info| info.scope.as_str());
    if source == "cli" || scope == "temporary" {
        "path"
    } else if scope == "user" {
        "user"
    } else if scope == "project" {
        "project"
    } else {
        "path"
    }
}

fn segments(path: &str) -> Vec<String> {
    let mut segments: Vec<String> = display_path(path)
        .replace('\\', "/")
        .split('/')
        .filter(|segment| !segment.is_empty() && *segment != "~")
        .map(str::to_owned)
        .collect();
    if segments.len() > 1
        && segments
            .last()
            .is_some_and(|last| last == "index.ts" || last == "index.js")
    {
        segments.pop();
    }
    segments
}

fn compact_labels(extensions: &[LoadedExtension]) -> Vec<String> {
    let all: Vec<Vec<String>> = extensions.iter().map(|e| segments(&e.path)).collect();
    all.iter()
        .enumerate()
        .map(|(index, own)| {
            for count in 1..=own.len() {
                let candidate = own[own.len() - count..].join("/");
                let unique = all.iter().enumerate().all(|(other, theirs)| {
                    other == index
                        || theirs[theirs.len().saturating_sub(count)..].join("/") != candidate
                });
                if unique {
                    return candidate;
                }
            }
            own.join("/")
        })
        .collect()
}

fn compact_list(labels: &[String]) -> String {
    let mut labels: Vec<&str> = labels
        .iter()
        .map(|label| label.trim())
        .filter(|label| !label.is_empty())
        .collect();
    labels.sort_by(|a, b| locale_order(a, b));
    fg("dim", &format!("  {}", labels.join(", ")))
}

fn scope_groups(extensions: &[LoadedExtension]) -> String {
    let mut lines = Vec::new();
    for group in ["project", "user", "path"] {
        let mut paths: Vec<&str> = extensions
            .iter()
            .filter(|extension| scope_group(extension) == group)
            .map(|extension| extension.path.as_str())
            .collect();
        if paths.is_empty() {
            continue;
        }
        lines.push(format!("  {}", fg("accent", group)));
        paths.sort_by(|a, b| locale_order(a, b));
        for path in paths {
            lines.push(fg("dim", &format!("    {}", extension_display_path(path))));
        }
    }
    lines.join("\n")
}

pub fn show_loaded(
    container: &Rc<RefCell<Container>>,
    extensions: &[LoadedExtension],
    expanded: bool,
) -> Vec<Rc<RefCell<ExpandableText>>> {
    container.borrow_mut().clear();
    let mut sections = Vec::new();
    if !extensions.is_empty() {
        let header = || fg("mdHeading", "[Extensions]");
        let labels = compact_labels(extensions);
        let listed = extensions.to_vec();
        let section = component(ExpandableText::new(
            Box::new(move || format!("{}\n{}", header(), compact_list(&labels))),
            Box::new(move || format!("{}\n{}", header(), scope_groups(&listed))),
            expanded,
            0,
            0,
        ));
        let mut list = container.borrow_mut();
        list.add_child(section.clone());
        list.add_child(component(Spacer::new(1)));
        sections.push(section);
    }
    sections
}
