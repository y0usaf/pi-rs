//! `/login` and `/logout` (upstream `components/oauth-selector.ts`, `login-dialog.ts` and
//! `auth-url.ts`): the auth-type question, the provider list with its search field, and the dialog
//! a running login shows its URL, device code and prompts in.

use pi_durable::ai::oauth::interaction::Prompt;
use pi_tui::components::input::{Input, InputOptions};
use pi_tui::fuzzy::fuzzy_filter;
use pi_tui::keybindings::get_keybindings;
use pi_tui::keys::matches_key;
use pi_tui::tui::Component;

use std::cell::RefCell;
use std::rc::Rc;

use pi_tui::tui::ComponentRef;

use super::{ActionSlot, OverlayAction, border, fit, key_hint, push, scroll_info, visible_range};
use crate::interactive::theme::{bold, fg};
use crate::login::{self, AuthType, Login, LoginRequest, Provider};

const MAX_VISIBLE: usize = 8;

/// The auth-type question `/login` asks first (upstream `showLoginAuthTypeSelector`).
pub struct MethodSelector {
    options: Vec<(&'static str, AuthType)>,
    selected: usize,
    /// The provider a login starts for directly, when the question was opened for one.
    provider: Option<String>,
    action: ActionSlot,
}

impl MethodSelector {
    pub fn new(action: ActionSlot) -> Self {
        let mut auths = Vec::new();
        for auth in [AuthType::Oauth, AuthType::ApiKey] {
            if login::has_auth(auth) {
                auths.push(auth);
            }
        }
        MethodSelector::for_provider(None, auths, action)
    }

    /// The same question limited to the auth types `auths` holds, remembered with the provider it
    /// belongs to.
    pub fn for_provider(provider: Option<&str>, auths: Vec<AuthType>, action: ActionSlot) -> Self {
        let mut options = Vec::new();
        if auths.contains(&AuthType::Oauth) {
            options.push(("Sign in with an account", AuthType::Oauth));
        }
        if auths.contains(&AuthType::ApiKey) {
            options.push(("Sign in with an API key", AuthType::ApiKey));
        }
        MethodSelector {
            options,
            selected: 0,
            provider: provider.map(str::to_owned),
            action,
        }
    }

    fn choose(&self) {
        let Some((_, auth)) = self.options.get(self.selected) else {
            return;
        };
        push(
            &self.action,
            OverlayAction::SetLoginMethod {
                auth: *auth,
                provider: self.provider.clone(),
            },
        );
    }
}

impl Component for MethodSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![
            border(width),
            String::new(),
            format!(" {}", bold(&fg("accent", "Select authentication method:"))),
            String::new(),
        ];
        for (index, (label, _)) in self.options.iter().enumerate() {
            let selected = index == self.selected;
            let prefix = if selected {
                fg("accent", "→ ")
            } else {
                "  ".to_owned()
            };
            let text = if selected {
                fg("accent", label)
            } else {
                fg("text", label)
            };
            lines.push(format!(" {prefix}{text}"));
        }
        lines.push(String::new());
        lines.push(format!(
            " {}  {}",
            key_hint("tui.select.confirm", "select"),
            key_hint("tui.select.cancel", "cancel")
        ));
        lines.push(String::new());
        lines.push(border(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.up") || matches_key(data, "k") {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.matches(data, "tui.select.down") || matches_key(data, "j") {
            self.selected = (self.selected + 1).min(self.options.len().saturating_sub(1));
        } else if kb.matches(data, "tui.select.confirm") || matches_key(data, "enter") {
            self.choose();
        } else if kb.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Close);
        }
    }
}

/// One row of the provider list.
#[derive(Clone)]
pub struct Row {
    pub id: String,
    pub name: String,
    pub auth: AuthType,
    pub subscription: Option<bool>,
    /// The auth method's name, which the list's search matches.
    pub method: String,
}

fn row_of(provider: &Provider) -> Row {
    Row {
        id: provider.id.to_owned(),
        name: provider.provider.to_owned(),
        auth: provider.auth,
        subscription: provider.subscription,
        method: provider.name.to_owned(),
    }
}

/// `formatAuthSelectorProviderStatus`: whether and how the provider is configured.
fn status_label(agent_dir: &str, row: &Row) -> String {
    match login::configured(agent_dir, &row.id) {
        None => fg("muted", " • not configured"),
        Some(auth) if auth == row.auth => fg("success", " ✓ configured"),
        Some(auth) => format!(
            "{}{}",
            fg("muted", " • "),
            fg(
                "warning",
                &format!("{} configured", auth.label(row.subscription))
            ),
        ),
    }
}

/// Everything the list's search matches a query against.
fn search_text(row: &Row) -> String {
    format!(
        "{} {} {} {}",
        row.name,
        row.id,
        row.auth.label(row.subscription),
        row.method
    )
}

/// `qOAuthSelectorComponent`: the provider list `/login` and `/logout` pick from.
pub struct AuthSelector {
    logout: bool,
    all: Vec<Row>,
    filtered: Vec<Row>,
    selected: usize,
    search: Input,
    agent_dir: String,
    action: ActionSlot,
}

impl AuthSelector {
    /// The login list for one auth type.
    pub fn new(
        auth: Option<AuthType>,
        agent_dir: &str,
        initial_search: Option<&str>,
        action: ActionSlot,
    ) -> Self {
        let all: Vec<Row> = login::providers_for(auth).into_iter().map(row_of).collect();
        let mut search = Input::new(InputOptions::default());
        if let Some(initial) = initial_search {
            search.set_value(initial);
        }
        let mut selector = AuthSelector {
            logout: false,
            all,
            filtered: Vec::new(),
            selected: 0,
            search,
            agent_dir: agent_dir.to_owned(),
            action,
        };
        selector.refilter();
        selector
    }

    /// The logout list: the providers with a stored credential (upstream
    /// `getLogoutProviderOptions`).
    pub fn logout(agent_dir: &str, action: ActionSlot) -> Result<Self, String> {
        let all: Vec<Row> = login::stored(agent_dir)?
            .into_iter()
            .map(|(id, name, auth)| Row {
                id,
                name,
                auth,
                subscription: None,
                method: String::new(),
            })
            .collect();
        let mut selector = AuthSelector {
            logout: true,
            all,
            filtered: Vec::new(),
            selected: 0,
            search: Input::new(InputOptions::default()),
            agent_dir: agent_dir.to_owned(),
            action,
        };
        selector.refilter();
        Ok(selector)
    }

    /// Whether the list has nothing to show, so `/logout` can say so without opening one.
    pub fn is_empty(&self) -> bool {
        self.all.is_empty()
    }

    fn refilter(&mut self) {
        self.filtered = fuzzy_filter(&self.all, self.search.value(), search_text);
        self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
    }

    fn choose(&self) {
        let Some(row) = self.filtered.get(self.selected) else {
            return;
        };
        let action = if self.logout {
            OverlayAction::LogoutProvider {
                provider: row.id.clone(),
                name: row.name.clone(),
                auth: row.auth,
            }
        } else {
            OverlayAction::LoginProvider {
                provider: row.id.clone(),
                auth: row.auth,
            }
        };
        push(&self.action, action);
    }
}

impl Component for AuthSelector {
    fn render(&mut self, width: usize) -> Vec<String> {
        let title = if self.logout {
            "Select provider to logout:"
        } else {
            "Select provider to configure:"
        };
        let mut distinct: Vec<AuthType> = self.all.iter().map(|row| row.auth).collect();
        distinct.sort_by_key(|auth| *auth as u8);
        distinct.dedup();
        let show_types = distinct.len() > 1;
        let mut lines = vec![
            border(width),
            String::new(),
            format!(" {}", bold(&fg("accent", title))),
            String::new(),
        ];
        lines.extend(self.search.render(width.saturating_sub(1)));
        lines.push(String::new());
        let (start, end) = visible_range(self.selected, self.filtered.len(), MAX_VISIBLE);
        for (position, row) in self.filtered[start..end].iter().enumerate() {
            let selected = start + position == self.selected;
            let status = status_label(&self.agent_dir, row);
            let types = if show_types {
                fg("muted", &format!(" [{}]", row.auth.label(row.subscription)))
            } else {
                String::new()
            };
            let line = if selected {
                format!(
                    " {} {}{types}{status}",
                    fg("accent", "→"),
                    fg("accent", &row.name)
                )
            } else {
                format!("   {}{types}{status}", fg("text", &row.name))
            };
            lines.push(line);
        }
        if start > 0 || end < self.filtered.len() {
            lines.push(scroll_info(self.selected, self.filtered.len()));
        }
        if self.filtered.is_empty() {
            let message = if self.all.is_empty() {
                if self.logout {
                    "No providers logged in. Use /login first."
                } else {
                    "No providers available"
                }
            } else {
                "No matching providers"
            };
            lines.push(format!("  {}", fg("muted", message)));
        }
        lines.push(String::new());
        lines.push(border(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.up") {
            self.selected = self.selected.saturating_sub(1);
        } else if kb.matches(data, "tui.select.down") {
            self.selected = (self.selected + 1).min(self.filtered.len().saturating_sub(1));
        } else if kb.matches(data, "tui.select.confirm") || matches_key(data, "enter") {
            self.choose();
        } else if kb.matches(data, "tui.select.cancel") {
            push(&self.action, OverlayAction::Close);
        } else {
            self.search.handle_input(data);
            self.refilter();
        }
    }
}

/// What the dialog is waiting for.
enum Pending {
    Text {
        id: u64,
        input: Input,
    },
    Select {
        id: u64,
        options: Vec<(String, String)>,
        selected: usize,
    },
}

/// `LoginDialogComponent`: the running login's URL, device code, progress and prompts.
pub struct LoginDialog {
    name: String,
    auth: AuthType,
    login: Login,
    lines: Vec<String>,
    pending: Option<Pending>,
    action: ActionSlot,
    finished: bool,
}

impl LoginDialog {
    pub fn new(
        provider: &str,
        name: &str,
        auth: AuthType,
        agent_dir: &str,
        action: ActionSlot,
    ) -> Self {
        let request = LoginRequest {
            provider: provider.to_owned(),
            auth,
        };
        LoginDialog {
            name: name.to_owned(),
            auth,
            login: Login::start(request, agent_dir),
            lines: Vec::new(),
            pending: None,
            action,
            finished: false,
        }
    }

    /// Applies everything the flow reported since the last frame; true when anything changed.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.login.events.try_recv() {
            changed = true;
            match event {
                login::UiEvent::AuthUrl { url, instructions } => {
                    self.lines.clear();
                    self.lines.push(fg("accent", &url));
                    if let Some(instructions) = instructions {
                        self.lines.push(fg("warning", &instructions));
                    }
                }
                login::UiEvent::DeviceCode {
                    user_code,
                    verification_uri,
                } => {
                    self.lines.push(fg("accent", &verification_uri));
                    self.lines
                        .push(fg("warning", &format!("Enter code: {user_code}")));
                }
                login::UiEvent::Progress(text) => self.lines.push(fg("dim", &text)),
                login::UiEvent::Prompt { id, prompt } => self.show_prompt(id, prompt),
                login::UiEvent::Dismiss(id) => {
                    if self.pending.as_ref().is_some_and(|p| p.id() == id) {
                        self.pending = None;
                    }
                }
                login::UiEvent::Done(result) => {
                    self.finished = true;
                    self.pending = None;
                    match result {
                        Ok(outcome) => push(&self.action, OverlayAction::LoggedIn(outcome)),
                        Err(message) => push(
                            &self.action,
                            OverlayAction::LoginFailed {
                                name: self.name.clone(),
                                auth: self.auth,
                                message,
                            },
                        ),
                    }
                }
            }
        }
        changed
    }

    fn show_prompt(&mut self, id: u64, prompt: Prompt) {
        match prompt {
            Prompt::Select { message, options } => {
                self.lines.push(String::new());
                self.lines.push(message);
                self.pending = Some(Pending::Select {
                    id,
                    options,
                    selected: 0,
                });
            }
            Prompt::Secret { message } | Prompt::Text { message } => {
                self.lines.push(String::new());
                self.lines.push(message);
                self.pending = Some(Pending::Text {
                    id,
                    input: Input::new(InputOptions::default()),
                });
            }
            Prompt::ManualCode {
                message,
                placeholder,
            } => {
                self.lines.push(String::new());
                self.lines.push(fg("dim", &message));
                let input = Input::new(InputOptions {
                    placeholder: Some(placeholder),
                    ..InputOptions::default()
                });
                self.pending = Some(Pending::Text { id, input });
            }
        }
    }
}

impl Pending {
    fn id(&self) -> u64 {
        match self {
            Pending::Text { id, .. } | Pending::Select { id, .. } => *id,
        }
    }
}

impl Component for LoginDialog {
    fn render(&mut self, width: usize) -> Vec<String> {
        let mut lines = vec![
            border(width),
            format!(
                " {}",
                bold(&fg("accent", &format!("Login to {}", self.name)))
            ),
            String::new(),
        ];
        for line in &self.lines {
            lines.extend(super::wrap(line, width.saturating_sub(1)));
        }
        if let Some(pending) = &mut self.pending {
            lines.push(String::new());
            match pending {
                Pending::Text { input, .. } => {
                    lines.extend(input.render(width.saturating_sub(1)));
                    lines.push(format!(" {}", key_hint("tui.select.cancel", "cancel")));
                }
                Pending::Select {
                    options, selected, ..
                } => {
                    for (index, (_, label)) in options.iter().enumerate() {
                        let chosen = index == *selected;
                        let prefix = if chosen {
                            fg("accent", "→ ")
                        } else {
                            "  ".to_owned()
                        };
                        let text = if chosen {
                            fg("accent", label)
                        } else {
                            fg("text", label)
                        };
                        lines.push(format!(" {prefix}{text}"));
                    }
                    lines.push(String::new());
                    lines.push(format!(
                        " {}  {}",
                        key_hint("tui.select.confirm", "select"),
                        key_hint("tui.select.cancel", "cancel")
                    ));
                }
            }
        } else if !self.finished {
            lines.push(String::new());
            lines.push(format!(" {}", fg("dim", "Waiting...")));
            lines.push(format!(" {}", key_hint("tui.select.cancel", "cancel")));
        }
        lines.push(String::new());
        lines.push(border(width));
        fit(lines, width)
    }

    fn accepts_input(&self) -> bool {
        true
    }

    fn is_focusable(&self) -> bool {
        true
    }

    fn handle_input(&mut self, data: &str) {
        let kb = get_keybindings();
        if kb.matches(data, "tui.select.cancel") {
            self.login.cancel();
            push(&self.action, OverlayAction::Close);
            return;
        }
        match self.pending.as_mut() {
            Some(Pending::Text { id, input }) => {
                if kb.matches(data, "tui.input.submit") || matches_key(data, "enter") {
                    let value = input.value().to_owned();
                    self.login.answer(*id, Some(value));
                    self.pending = None;
                } else {
                    input.handle_input(data);
                }
            }
            Some(Pending::Select {
                id,
                options,
                selected,
            }) => {
                if kb.matches(data, "tui.select.up") || matches_key(data, "k") {
                    *selected = selected.saturating_sub(1);
                } else if kb.matches(data, "tui.select.down") || matches_key(data, "j") {
                    *selected = (*selected + 1).min(options.len().saturating_sub(1));
                } else if kb.matches(data, "tui.select.confirm") || matches_key(data, "enter") {
                    let choice = options.get(*selected).map(|(option, _)| option.clone());
                    let id = *id;
                    self.pending = None;
                    self.login.answer(id, choice);
                }
            }
            None => {}
        }
    }
}

/// Opens the dialog for one provider and hands the loop both the component to mount and the handle
/// it polls for the flow's progress.
pub fn start(
    agent_dir: &str,
    provider: &str,
    name: &str,
    auth: AuthType,
    slot: &ActionSlot,
) -> (ComponentRef, Rc<RefCell<LoginDialog>>) {
    let dialog: Rc<RefCell<LoginDialog>> = Rc::new(RefCell::new(LoginDialog::new(
        provider,
        name,
        auth,
        agent_dir,
        slot.clone(),
    )));
    let built: ComponentRef = dialog.clone();
    (built, dialog)
}
