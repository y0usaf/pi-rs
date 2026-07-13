const RAW_PLACEHOLDER = /\{\{\{\s*([A-Za-z0-9_]+)\s*\}\}\}/g;
const ESCAPED_PLACEHOLDER = /\{\{\s*([A-Za-z0-9_]+)\s*\}\}/g;

function stringValue(value: unknown): string {
  if (value === undefined || value === null) return "";
  return String(value);
}

export function xmlEscape(value: unknown): string {
  return stringValue(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

export function renderTemplate(template: string, vars: Record<string, unknown>): string {
  const withRaw = template.replace(RAW_PLACEHOLDER, (_match, key: string) => stringValue(vars[key]));
  return withRaw.replace(ESCAPED_PLACEHOLDER, (_match, key: string) => xmlEscape(vars[key]));
}

export function splitPromptBlock(block: string): string[] {
  return block
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
}
