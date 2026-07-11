import { readFileSync } from "node:fs";
import { join } from "node:path";
import type { AuthStatus, AuthStorage } from "../../ref/pi/packages/coding-agent/src/core/auth-storage.ts";
import { KeybindingsManager } from "../../ref/pi/packages/coding-agent/src/core/keybindings.ts";
import { BUILT_IN_PROVIDER_DISPLAY_NAMES } from "../../ref/pi/packages/coding-agent/src/core/provider-display-names.ts";
import { ExtensionSelectorComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/extension-selector.ts";
import { LoginDialogComponent } from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/login-dialog.ts";
import {
  type AuthSelectorProvider,
  OAuthSelectorComponent,
} from "../../ref/pi/packages/coding-agent/src/modes/interactive/components/oauth-selector.ts";
import { initTheme, theme } from "../../ref/pi/packages/coding-agent/src/modes/interactive/theme/theme.ts";
import { Spacer } from "../../ref/pi/packages/tui/src/components/spacer.ts";
import { Text } from "../../ref/pi/packages/tui/src/components/text.ts";
import { setKeybindings } from "../../ref/pi/packages/tui/src/keybindings.ts";
import { Container, CURSOR_MARKER, TUI } from "../../ref/pi/packages/tui/src/tui.ts";
import type { Terminal } from "../../ref/pi/packages/tui/src/terminal.ts";

type LoginEvent = {
  type: string;
  url?: string;
  instructions?: string;
  message?: string;
  placeholder?: string;
};
type Step = { name: string; show?: "login" | "logout"; emit?: LoginEvent[]; input?: string[] };
type Scenario = {
  columns: number;
  rows: number;
  authPath: string;
  docsPath: string;
  model: { id: string; provider: string };
  oauthProviders: Array<{ id: string; name: string; usesCallbackServer?: boolean }>;
  loginProviders: Record<string, AuthSelectorProvider[]>;
  credentials: Record<string, { type: "oauth" | "api_key"; key?: string }>;
  authStatus?: Record<string, AuthStatus>;
  steps: Step[];
};

class CaptureTerminal implements Terminal {
  private input?: (data: string) => void; private resized?: () => void; private chunks: string[] = [];
  kittyProtocolActive = true;
  constructor(public columns: number, public rows: number) {}
  start(input: (data: string) => void, resized: () => void): void { this.input = input; this.resized = resized; }
  async drainInput(): Promise<void> {} stop(): void {}
  write(data: string): void { this.chunks.push(data); }
  moveBy(lines: number): void { if (lines > 0) this.write(`\x1b[${lines}B`); else if (lines < 0) this.write(`\x1b[${-lines}A`); }
  hideCursor(): void { this.write("\x1b[?25l"); } showCursor(): void { this.write("\x1b[?25h"); }
  clearLine(): void { this.write("\x1b[K"); } clearFromCursor(): void { this.write("\x1b[J"); }
  clearScreen(): void { this.write("\x1b[2J\x1b[H"); } setTitle(): void {} setProgress(): void {}
  send(data: string): void { this.input?.(data); }
  resize(columns: number, rows: number): void { this.columns = columns; this.rows = rows; this.resized?.(); }
  take(): string { const result = this.chunks.join(""); this.chunks = []; return result; }
}

const scenario = JSON.parse(readFileSync(process.argv[2]!, "utf8")) as Scenario;
// openBrowser must not launch anything during oracle generation; its spawn
// resolves the handler through PATH and swallows launcher failures.
process.env.PATH = "/nonexistent-pi-rs-ui-parity";
setKeybindings(new KeybindingsManager());
initTheme("dark", false);

const terminal = new CaptureTerminal(scenario.columns, scenario.rows);
const ui = new TUI(terminal, true);

const chatContainer = new Container();
const editorContainer = new Container();
let editorValue = "";
const editor = {
  focused: false,
  invalidate() {},
  handleInput(data: string) { if (data === "\r") editorValue = ""; else editorValue += data; },
  render() { return [theme.fg("accent", editorValue) + (editor.focused ? CURSOR_MARKER : "")]; },
};
ui.addChild(chatContainer);
ui.addChild(editorContainer);
editorContainer.addChild(editor);
ui.setFocus(editor);
ui.start();

const credentials = scenario.credentials ?? {};
const authStorage = { get: (id: string) => credentials[id] } as unknown as AuthStorage;
const getAuthStatus = (id: string): AuthStatus => scenario.authStatus?.[id] ?? { configured: false };

// interactive-mode.ts UI helper ports over the harness containers.
let lastStatusSpacer: Spacer | undefined;
let lastStatusText: Text | undefined;
function showStatus(message: string): void {
  const children = chatContainer.children;
  const last = children.length > 0 ? children[children.length - 1] : undefined;
  const secondLast = children.length > 1 ? children[children.length - 2] : undefined;
  if (last && secondLast && last === lastStatusText && secondLast === lastStatusSpacer) {
    lastStatusText!.setText(theme.fg("dim", message));
    ui.requestRender();
    return;
  }
  const spacer = new Spacer(1);
  const text = new Text(theme.fg("dim", message), 1, 0);
  chatContainer.addChild(spacer);
  chatContainer.addChild(text);
  lastStatusSpacer = spacer;
  lastStatusText = text;
  ui.requestRender();
}
function showError(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("error", `Error: ${message}`), 1, 0));
  chatContainer.addChild(new Spacer(1));
  ui.requestRender();
}
function showWarning(message: string): void {
  chatContainer.addChild(new Spacer(1));
  chatContainer.addChild(new Text(theme.fg("warning", `Warning: ${message}`), 1, 0));
  ui.requestRender();
}

function restoreEditor(): void {
  editorContainer.clear();
  editorContainer.addChild(editor);
  ui.setFocus(editor);
  ui.requestRender();
}

// interactive-mode.ts showSelector.
function showSelector(create: (done: () => void) => { component: unknown; focus: unknown }): void {
  const { component, focus } = create(() => restoreEditor());
  editorContainer.clear();
  editorContainer.addChild(component as never);
  ui.setFocus(focus as never);
  ui.requestRender();
}

const ANTHROPIC_SUBSCRIPTION_AUTH_WARNING =
  "Anthropic subscription auth is active. Third-party harness usage draws from extra usage and is billed per token, not your Claude plan limits. Manage extra usage at https://claude.ai/settings/usage.";
let anthropicSubscriptionWarningShown = false;
function maybeWarnAboutAnthropicSubscriptionAuth(): void {
  const model = scenario.model;
  if (anthropicSubscriptionWarningShown) return;
  if (!model || model.provider !== "anthropic") return;
  const stored = credentials["anthropic"];
  if (stored?.type === "oauth") {
    anthropicSubscriptionWarningShown = true;
    showWarning(ANTHROPIC_SUBSCRIPTION_AUTH_WARNING);
    return;
  }
  const apiKey = stored?.type === "api_key" ? stored.key : undefined;
  if (typeof apiKey === "string" && apiKey.startsWith("sk-ant-oat")) {
    anthropicSubscriptionWarningShown = true;
    showWarning(ANTHROPIC_SUBSCRIPTION_AUTH_WARNING);
  }
}

function completeProviderAuthentication(providerName: string, authType: "oauth" | "api_key"): void {
  const actionLabel = authType === "oauth" ? `Logged in to ${providerName}` : `Saved API key for ${providerName}`;
  showStatus(`${actionLabel}. Credentials saved to ${scenario.authPath}`);
  maybeWarnAboutAnthropicSubscriptionAuth();
}

function getProviderDisplayName(provider: string): string {
  const oauthProvider = scenario.oauthProviders.find((p) => p.id === provider);
  return oauthProvider?.name ?? BUILT_IN_PROVIDER_DISPLAY_NAMES[provider] ?? provider;
}

function getLogoutProviderOptions(): AuthSelectorProvider[] {
  const options: AuthSelectorProvider[] = [];
  for (const providerId of Object.keys(credentials)) {
    const credential = credentials[providerId];
    if (!credential) continue;
    options.push({ id: providerId, name: getProviderDisplayName(providerId), authType: credential.type });
  }
  return options.sort((a, b) => a.name.localeCompare(b.name));
}

// The mounted login dialog and its flow context; scenario `emit` events
// stand in for the OAuth flow's callbacks.
let currentDialog: LoginDialogComponent | undefined;
let currentProvider: { id: string; name: string } | undefined;

const BEDROCK_PROVIDER_ID = "amazon-bedrock";

function showLoginDialog(providerId: string, providerName: string): void {
  const dialog = new LoginDialogComponent(ui, providerId, () => {}, providerName);
  editorContainer.clear();
  editorContainer.addChild(dialog);
  ui.setFocus(dialog);
  ui.requestRender();
  currentDialog = dialog;
  currentProvider = { id: providerId, name: providerName };
}

function showApiKeyLoginDialog(providerId: string, providerName: string): void {
  const dialog = new LoginDialogComponent(ui, providerId, () => {}, providerName);
  editorContainer.clear();
  editorContainer.addChild(dialog);
  ui.setFocus(dialog);
  ui.requestRender();
  dialog
    .showPrompt("Enter API key:")
    .then((value) => {
      const apiKey = value.trim();
      if (!apiKey) throw new Error("API key cannot be empty.");
      credentials[providerId] = { type: "api_key", key: apiKey };
      restoreEditor();
      completeProviderAuthentication(providerName, "api_key");
    })
    .catch((error: unknown) => {
      restoreEditor();
      const errorMsg = error instanceof Error ? error.message : String(error);
      if (errorMsg !== "Login cancelled") {
        showError(`Failed to save API key for ${providerName}: ${errorMsg}`);
      }
    });
}

function showBedrockSetupDialog(providerId: string, providerName: string): void {
  const dialog = new LoginDialogComponent(ui, providerId, () => restoreEditor(), providerName, "Amazon Bedrock setup");
  dialog.showInfo([
    theme.fg("text", "Amazon Bedrock uses AWS credentials instead of a single API key."),
    theme.fg("text", "Configure an AWS profile, IAM keys, bearer token, or role-based credentials."),
    theme.fg("muted", "See:"),
    theme.fg("accent", `  ${join(scenario.docsPath, "providers.md")}`),
  ]);
  editorContainer.clear();
  editorContainer.addChild(dialog);
  ui.setFocus(dialog);
  ui.requestRender();
}

function showLoginProviderSelector(authType: "oauth" | "api_key"): void {
  const providerOptions = scenario.loginProviders[authType] ?? [];
  if (providerOptions.length === 0) {
    showStatus(authType === "oauth" ? "No subscription providers available." : "No API key providers available.");
    return;
  }
  showSelector((done) => {
    const selector = new OAuthSelectorComponent(
      "login",
      authStorage,
      providerOptions,
      (providerId: string) => {
        done();
        const providerOption = providerOptions.find((provider) => provider.id === providerId);
        if (!providerOption) return;
        if (providerOption.authType === "oauth") {
          showLoginDialog(providerOption.id, providerOption.name);
        } else if (providerOption.id === BEDROCK_PROVIDER_ID) {
          showBedrockSetupDialog(providerOption.id, providerOption.name);
        } else {
          showApiKeyLoginDialog(providerOption.id, providerOption.name);
        }
      },
      () => {
        done();
        showLoginAuthTypeSelector();
      },
      getAuthStatus,
    );
    return { component: selector, focus: selector };
  });
}

function showLoginAuthTypeSelector(): void {
  const subscriptionLabel = "Use a subscription";
  const apiKeyLabel = "Use an API key";
  showSelector((done) => {
    const selector = new ExtensionSelectorComponent(
      "Select authentication method:",
      [subscriptionLabel, apiKeyLabel],
      (option) => {
        done();
        const authType = option === subscriptionLabel ? "oauth" : "api_key";
        showLoginProviderSelector(authType);
      },
      () => {
        done();
        ui.requestRender();
      },
    );
    return { component: selector, focus: selector };
  });
}

function showOAuthSelector(mode: "login" | "logout"): void {
  if (mode === "login") {
    showLoginAuthTypeSelector();
    return;
  }
  const providerOptions = getLogoutProviderOptions();
  if (providerOptions.length === 0) {
    showStatus(
      "No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.",
    );
    return;
  }
  showSelector((done) => {
    const selector = new OAuthSelectorComponent(
      mode,
      authStorage,
      providerOptions,
      (providerId: string) => {
        done();
        const providerOption = providerOptions.find((provider) => provider.id === providerId);
        if (!providerOption) return;
        delete credentials[providerOption.id];
        const message =
          providerOption.authType === "oauth"
            ? `Logged out of ${providerOption.name}`
            : `Removed stored API key for ${providerOption.name}. Environment variables and models.json config are unchanged.`;
        showStatus(message);
      },
      () => {
        done();
        ui.requestRender();
      },
      getAuthStatus,
    );
    return { component: selector, focus: selector };
  });
}

// interactive-mode.ts showLoginDialog's OAuthLoginCallbacks bodies, driven
// by scenario events instead of the live flow.
function emitLoginEvent(event: LoginEvent): void {
  const dialog = currentDialog;
  const provider = currentProvider;
  if (!dialog || !provider) return;
  const usesCallbackServer =
    scenario.oauthProviders.find((p) => p.id === provider.id)?.usesCallbackServer ?? false;
  switch (event.type) {
    case "auth": {
      dialog.showAuth(event.url!, event.instructions);
      if (usesCallbackServer) {
        dialog
          .showManualInput("Paste redirect URL below, or complete login in browser:")
          .then(() => {})
          .catch(() => {});
      }
      break;
    }
    case "prompt": {
      dialog
        .showPrompt(event.message!, event.placeholder)
        .then(() => {})
        .catch(() => {});
      break;
    }
    case "progress": {
      dialog.showProgress(event.message!);
      break;
    }
    case "done": {
      credentials[provider.id] = { type: "oauth" };
      currentDialog = undefined;
      currentProvider = undefined;
      restoreEditor();
      completeProviderAuthentication(provider.name, "oauth");
      break;
    }
    case "error": {
      currentDialog = undefined;
      currentProvider = undefined;
      restoreEditor();
      if (event.message !== "Login cancelled") {
        showError(`Failed to login to ${provider.name}: ${event.message}`);
      }
      break;
    }
  }
}

const frames: Array<{ name: string; columns: number; rows: number; ansi: string }> = [];
async function capture(name: string, force = false) {
  ui.requestRender(force);
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
  frames.push({ name, columns: terminal.columns, rows: terminal.rows, ansi: terminal.take() });
}

async function main() {
  await capture("startup", true);
  for (const step of scenario.steps) {
    if (step.show) showOAuthSelector(step.show);
    for (const event of step.emit ?? []) emitLoginEvent(event);
    for (const data of step.input ?? []) {
      terminal.send(data);
      // Let promise continuations (input submit -> then chains) settle.
      await new Promise<void>((resolve) => setTimeout(resolve, 0));
    }
    await capture(step.name);
  }
  ui.stop();
  process.stdout.write(JSON.stringify({ frames }));
}

main().catch((error) => { console.error(error); process.exitCode = 1; });
