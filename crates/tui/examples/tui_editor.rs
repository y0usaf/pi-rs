//! Editor demo: type, Enter submits (shown above), / for commands, Tab for paths, Ctrl+C quits.

use pi_tui::autocomplete::{CombinedAutocompleteProvider, CommandEntry, SlashCommand};
use pi_tui::components::editor::{Editor, EditorOptions, EditorTheme};
use pi_tui::components::select_list::SelectListTheme;
use pi_tui::components::text::Text;
use pi_tui::keys::matches_key;
use pi_tui::terminal::{ProcessTerminal, Terminal};
use pi_tui::tui::{InputListenerResult, component};
use pi_tui::tui_main_screen::{TuiMainScreen, as_ref};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Instant;

fn sgr(code: &'static str) -> Rc<dyn Fn(&str) -> String> {
    Rc::new(move |s: &str| format!("\x1b[{code}m{s}\x1b[0m"))
}

fn cmd(name: &str, description: &str) -> CommandEntry {
    CommandEntry::Command(SlashCommand {
        name: name.to_string(),
        description: Some(description.to_string()),
        argument_hint: None,
        argument_completions: None,
    })
}

fn main() {
    let start = Instant::now();
    let now = || start.elapsed().as_millis() as u64;
    let mut tui = TuiMainScreen::new(ProcessTerminal::new());

    let header = component(Text::new(
        "pi-tui editor demo. Enter submits, Shift+Enter new line, / commands, Tab paths, Ctrl+C quits.",
        1,
        0,
    ));
    let log = component(Text::new("", 1, 0));
    let theme = EditorTheme {
        border_color: sgr("90"),
        select_list: SelectListTheme {
            selected_prefix: sgr("36"),
            selected_text: sgr("36"),
            description: sgr("90"),
            scroll_info: sgr("90"),
            no_match: sgr("90"),
        },
    };
    let editor = component(Editor::new(
        theme,
        EditorOptions {
            padding_x: Some(1),
            autocomplete_max_visible: None,
        },
    ));
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let commands = vec![
        cmd("clear", "Clear the submitted log"),
        cmd("quit", "Leave the demo"),
    ];
    let submitted: Rc<RefCell<Vec<String>>> = Rc::default();
    let dirty = Rc::new(Cell::new(false));
    {
        let mut e = editor.borrow_mut();
        e.set_autocomplete_provider(Rc::new(CombinedAutocompleteProvider::new(
            commands, &cwd, None,
        )));
        let (s, d) = (submitted.clone(), dirty.clone());
        e.on_submit = Some(Box::new(move |t: &str| s.borrow_mut().push(t.to_string())));
        e.request_render = Some(Rc::new(move || d.set(true)));
    }

    let quit = Rc::new(Cell::new(false));
    let q = quit.clone();
    tui.add_input_listener(Box::new(move |data: &str| {
        if matches_key(data, "ctrl+c") {
            q.set(true);
            return Some(InputListenerResult {
                consume: true,
                data: None,
            });
        }
        None
    }));
    tui.add_child(as_ref(&header));
    tui.add_child(as_ref(&log));
    tui.add_child(as_ref(&editor));
    tui.set_focus(Some(as_ref(&editor)));
    tui.start();

    let mut history: Vec<String> = Vec::new();
    while !quit.get() {
        let due = tui.tick(now());
        let timeout = due.map(|d| d.saturating_sub(now()));
        let events = tui.terminal.poll(timeout);
        tui.handle_events(events);
        for t in submitted.borrow_mut().drain(..) {
            match t.trim() {
                "/quit" => quit.set(true),
                "/clear" => history.clear(),
                _ => {
                    editor.borrow_mut().add_to_history(&t);
                    history.push(format!("> {}", t.replace('\n', "\n  ")));
                }
            }
            log.borrow_mut().set_text(&history.join("\n"));
            dirty.set(true);
        }
        if dirty.replace(false) {
            tui.request_render(false);
        }
    }
    tui.stop(false);
    tui.terminal.drain_input(1000, 50);
}
