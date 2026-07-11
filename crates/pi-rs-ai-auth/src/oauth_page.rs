//! Port of `utils/oauth/oauth-page.ts` — the HTML pages served by the
//! OAuth callback server. Markup and styles are verbatim from the spec
//! (the page a user sees after browser login is part of the 1:1 test).

const LOGO_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 800 800" aria-hidden="true"><path fill="#fff" fill-rule="evenodd" d="M165.29 165.29 H517.36 V400 H400 V517.36 H282.65 V634.72 H165.29 Z M282.65 282.65 V400 H400 V282.65 Z"/><path fill="#fff" d="M517.36 400 H634.72 V634.72 H517.36 Z"/></svg>"##;

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

struct PageOptions<'a> {
    title: &'a str,
    heading: &'a str,
    message: &'a str,
    details: Option<&'a str>,
}

fn render_page(options: PageOptions<'_>) -> String {
    let title = escape_html(options.title);
    let heading = escape_html(options.heading);
    let message = escape_html(options.message);
    let details = options.details.map(escape_html);

    let details_block = details
        .map(|details| format!(r#"<div class="details">{details}</div>"#))
        .unwrap_or_default();

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>{title}</title>
  <style>
    :root {{
      --text: #fafafa;
      --text-dim: #a1a1aa;
      --page-bg: #09090b;
      --font-sans: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, "Noto Sans", sans-serif, "Apple Color Emoji", "Segoe UI Emoji", "Segoe UI Symbol", "Noto Color Emoji";
      --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
    }}
    * {{ box-sizing: border-box; }}
    html {{ color-scheme: dark; }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: flex;
      align-items: center;
      justify-content: center;
      padding: 24px;
      background: var(--page-bg);
      color: var(--text);
      font-family: var(--font-sans);
      text-align: center;
    }}
    main {{
      width: 100%;
      max-width: 560px;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: center;
    }}
    .logo {{
      width: 72px;
      height: 72px;
      display: block;
      margin-bottom: 24px;
    }}
    h1 {{
      margin: 0 0 10px;
      font-size: 28px;
      line-height: 1.15;
      font-weight: 650;
      color: var(--text);
    }}
    p {{
      margin: 0;
      line-height: 1.7;
      color: var(--text-dim);
      font-size: 15px;
    }}
    .details {{
      margin-top: 16px;
      font-family: var(--font-mono);
      font-size: 13px;
      color: var(--text-dim);
      white-space: pre-wrap;
      word-break: break-word;
    }}
  </style>
</head>
<body>
  <main>
    <div class="logo">{LOGO_SVG}</div>
    <h1>{heading}</h1>
    <p>{message}</p>
    {details_block}
  </main>
</body>
</html>"#
    )
}

/// Spec: `oauthSuccessHtml(message)`.
pub fn oauth_success_html(message: &str) -> String {
    render_page(PageOptions {
        title: "Authentication successful",
        heading: "Authentication successful",
        message,
        details: None,
    })
}

/// Spec: `oauthErrorHtml(message, details?)`.
pub fn oauth_error_html(message: &str, details: Option<&str>) -> String {
    render_page(PageOptions {
        title: "Authentication failed",
        heading: "Authentication failed",
        message,
        details,
    })
}
