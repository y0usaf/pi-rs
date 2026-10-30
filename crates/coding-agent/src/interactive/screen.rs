use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use pi_tui::components::scroll_view::{
    Overscroll, ScrollOptions, ScrollRef, ScrollView, Scrollbar,
};
use pi_tui::components::stack::{Stack, StackEntry};
use pi_tui::terminal::{ProcessTerminal, Terminal, TerminalEvent};
use pi_tui::tui::{ComponentRef, TuiCore};
use pi_tui::tui_alt_screen::{AltScreenOptions, TuiAltScreen};
use pi_tui::tui_main_screen::TuiMainScreen;
use pi_tui::wheel_scroll::WheelScrollLines;

use super::selectors::key_display;
use super::theme;
use crate::settings::{SettingsManager, WheelScrollLines as SettingLines};

pub struct Viewport {
    pub root: ComponentRef,
    pub transcript: ScrollRef,
}

pub struct ViewportParts {
    pub document: ComponentRef,
    pub pending: ComponentRef,
    pub status: ComponentRef,
    pub widgets_above: ComponentRef,
    pub editor: ComponentRef,
    pub widgets_below: ComponentRef,
    pub footer: ComponentRef,
}

impl ViewportParts {
    pub fn components(&self) -> Vec<ComponentRef> {
        vec![
            self.document.clone(),
            self.pending.clone(),
            self.status.clone(),
            self.widgets_above.clone(),
            self.editor.clone(),
            self.widgets_below.clone(),
            self.footer.clone(),
        ]
    }
}

pub fn chat_viewport(parts: &ViewportParts, scrollbar: &str) -> Viewport {
    let transcript = ScrollView::new(
        parts.document.clone(),
        ScrollOptions {
            follow_end: true,
            primary: true,
            overscroll: Overscroll::Chain,
            scrollbar: Scrollbar::parse(scrollbar),
            track_style: theme::fg_fn("scrollbarTrack"),
            thumb_style: theme::fg_fn("scrollbarThumb"),
            ..ScrollOptions::default()
        },
    );
    let state = transcript.state.clone();
    let dock = Stack::vertical(vec![
        StackEntry::new(parts.pending.clone()).min_size(0),
        StackEntry::new(parts.status.clone()).min_size(0),
        StackEntry::new(parts.widgets_above.clone()).min_size(0),
        StackEntry::new(parts.editor.clone()).min_size(3),
        StackEntry::new(parts.widgets_below.clone()).min_size(0),
        StackEntry::new(parts.footer.clone()).min_size(0),
    ]);
    let transcript: ComponentRef = Rc::new(RefCell::new(transcript));
    let dock: ComponentRef = Rc::new(RefCell::new(dock));
    Viewport {
        root: Rc::new(RefCell::new(Stack::vertical(vec![
            StackEntry::new(transcript).basis(0).grow(1).min_size(1),
            StackEntry::new(dock).grow(0).min_size(1),
        ]))),
        transcript: state,
    }
}

fn wheel_lines(settings: &SettingsManager) -> WheelScrollLines {
    match settings.fullscreen_wheel_scroll_lines() {
        SettingLines::Auto => WheelScrollLines::Auto,
        SettingLines::Lines(lines) => WheelScrollLines::Lines(f64::from(lines)),
    }
}

fn alt_options(settings: &SettingsManager) -> AltScreenOptions {
    AltScreenOptions {
        mouse: true,
        wheel_scroll_lines: wheel_lines(settings),
        scroll_to_end_indicator: Some(Rc::new(|| {
            let shortcut = key_display("tui.altScreen.bottom");
            let label = if shortcut.is_empty() {
                " ↓ Jump to latest message ".to_owned()
            } else {
                format!(" ↓ Jump to latest message · {shortcut} ")
            };
            theme::bg("selectedBg", &theme::fg("text", &label))
        })),
    }
}

pub enum Screen {
    Main(Box<TuiMainScreen<ProcessTerminal>>),
    Alt(Box<TuiAltScreen<ProcessTerminal>>),
}

impl Deref for Screen {
    type Target = TuiCore;
    fn deref(&self) -> &TuiCore {
        match self {
            Screen::Main(tui) => &tui.core,
            Screen::Alt(tui) => &tui.core,
        }
    }
}

impl DerefMut for Screen {
    fn deref_mut(&mut self) -> &mut TuiCore {
        match self {
            Screen::Main(tui) => &mut tui.core,
            Screen::Alt(tui) => &mut tui.core,
        }
    }
}

impl Screen {
    pub fn new(
        fullscreen: bool,
        settings: &SettingsManager,
        components: &[ComponentRef],
        layout_root: &ComponentRef,
    ) -> Screen {
        let mut core = TuiCore::default();
        for component in components {
            core.add_child(component.clone());
        }
        Screen::mounted(
            fullscreen,
            settings,
            core,
            ProcessTerminal::new(),
            layout_root,
        )
    }

    fn mounted(
        fullscreen: bool,
        settings: &SettingsManager,
        core: TuiCore,
        terminal: ProcessTerminal,
        layout_root: &ComponentRef,
    ) -> Screen {
        if fullscreen {
            let mut tui = TuiAltScreen::from_parts(core, terminal, alt_options(settings));
            tui.set_layout_root(Some(layout_root.clone()));
            Screen::Alt(Box::new(tui))
        } else {
            Screen::Main(Box::new(TuiMainScreen::from_parts(core, terminal)))
        }
    }

    pub fn is_fullscreen(&self) -> bool {
        matches!(self, Screen::Alt(_))
    }

    pub fn terminal(&mut self) -> &mut ProcessTerminal {
        match self {
            Screen::Main(tui) => &mut tui.terminal,
            Screen::Alt(tui) => &mut tui.terminal,
        }
    }

    pub fn rows(&mut self) -> usize {
        self.terminal().rows()
    }

    pub fn start(&mut self) {
        match self {
            Screen::Main(tui) => tui.start(),
            Screen::Alt(tui) => tui.start(),
        }
    }

    pub fn stop(&mut self, preserve_screen: bool) {
        match self {
            Screen::Main(tui) => tui.stop(preserve_screen),
            Screen::Alt(tui) => tui.stop(preserve_screen),
        }
    }

    pub fn tick(&mut self, now: u64) -> Option<u64> {
        match self {
            Screen::Main(tui) => tui.tick(now),
            Screen::Alt(tui) => tui.tick(now),
        }
    }

    pub fn render_now(&mut self, force: bool, now: u64) {
        match self {
            Screen::Main(tui) => tui.render_now(force, now),
            Screen::Alt(tui) => tui.render_now(force, now),
        }
    }

    pub fn handle_events(&mut self, events: Vec<TerminalEvent>) {
        match self {
            Screen::Main(tui) => tui.handle_events(events),
            Screen::Alt(tui) => tui.handle_events(events),
        }
    }

    pub fn set_title(&mut self, title: &str) {
        self.terminal().set_title(title);
    }

    fn switch(
        self,
        fullscreen: bool,
        settings: &SettingsManager,
        layout_root: &ComponentRef,
    ) -> Screen {
        if fullscreen == self.is_fullscreen() || self.has_overlay() {
            return self;
        }
        let mut previous = self;
        previous.stop(true);
        let (mut core, terminal) = match previous {
            Screen::Main(tui) => (*tui).into_parts(),
            Screen::Alt(tui) => (*tui).into_parts(),
        };
        let focus = core.focused_component();
        core.set_focus(None);
        let mut next = Screen::mounted(fullscreen, settings, core, terminal, layout_root);
        next.invalidate();
        next.set_focus(focus);
        next
    }

    pub fn stop_interactive(
        self,
        exit_output: &str,
        settings: &SettingsManager,
        layout_root: &ComponentRef,
        now: u64,
    ) -> Screen {
        let mut screen = self;
        if screen.is_fullscreen() && exit_output == "transcript" {
            while screen.has_overlay() {
                screen.hide_overlay();
            }
            screen = screen.switch(false, settings, layout_root);
            screen.render_now(false, now);
        }
        let preserve = screen.is_fullscreen();
        screen.stop(preserve);
        screen
    }
}
