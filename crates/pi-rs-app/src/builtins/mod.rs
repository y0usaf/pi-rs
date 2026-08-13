//! Embedded builtin packs — first-party behavior shipped as Lua through
//! the public extension surface (DESIGN divergence 2; locked decision:
//! embedded `.lua` via `include_str!`, loaded through the same path as
//! on-disk extensions).
//!
//! The tools pack is the `core/tools/` port (spec: pi v0.79.0), one
//! fragment file per spec module concatenated into a single chunk so the
//! fragments share the chunk-local helpers (`prelude.lua`) while parity
//! audits stay file-shaped. Fragment order is dependency order followed by
//! the spec's `createAllTools` registration order.

pub mod manifest;

use pi_rs_host::EmbeddedPack;

/// Exact landed core of the default interactive frontend. Theme assets and
/// policy are Lua-authored and loaded through the same extension API as users.
///
/// The shared agent-policy fragments (messages, branch-summary, compaction,
/// system-prompt, agent-session, bash-executor) are NOT concatenated here:
/// they are defined once as public exact-version modules by the always-loaded
/// `agent-core` pack (`pi.agent.*`), and this frontend requires + rebinds them
/// at the top of `interactive.lua`. syntax-highlight stays a shared chunk-local
/// included by both the tools and interactive packs (legacy cross-pack
/// fragment, tracked separately).
pub const INTERACTIVE_PACK: EmbeddedPack = EmbeddedPack {
    name: "interactive",
    source: concat!(
        "local DARK_THEME = (function()\n",
        include_str!("interactive/theme/dark.lua"),
        "\nend)()\nlocal LIGHT_THEME = (function()\n",
        include_str!("interactive/theme/light.lua"),
        "\nend)()\nDAX_HEX = [==[",
        include_str!("interactive/assets/daxnuts.hex"),
        "]==]\nCLANKOLAS_BASE64 = [==[",
        include_str!("interactive/assets/clankolas.base64"),
        "]==]\nCHANGELOG_MD = [=====[",
        include_str!("interactive/changelog.md"),
        "]=====]\ndo\nlocal EXPORT_TEMPLATE_HTML = [=====[",
        include_str!("interactive/export-html/template.html"),
        "]=====]\nlocal EXPORT_TEMPLATE_CSS = [=====[",
        include_str!("interactive/export-html/template.css"),
        "]=====]\nlocal EXPORT_TEMPLATE_JS = [=====[",
        include_str!("interactive/export-html/template.js"),
        "]=====]\nlocal EXPORT_MARKED_JS = [=====[",
        include_str!("interactive/export-html/vendor/marked.min.js"),
        "]=====]\nlocal EXPORT_HIGHLIGHT_JS = [=====[",
        include_str!("interactive/export-html/vendor/highlight.min.js"),
        "]=====]\n",
        include_str!("utils/export-html.lua"),
        "\nend\n",
        include_str!("utils/syntax-highlight.lua"),
        "\n",
        include_str!("utils/prompt-templates.lua"),
        "\n",
        include_str!("utils/skills.lua"),
        "\n",
        include_str!("utils/package-manager.lua"),
        "\n",
        include_str!("utils/resources.lua"),
        "\n",
        include_str!("utils/extensions.lua"),
        "\n",
        include_str!("interactive.lua"),
    ),
};

/// First-party agent-policy substrate pack: the shared cross-pack helper
/// fragments converted to public exact-version modules under `pi.agent.*`
/// (PLAN 9.7/9.10). Owned once here so the interactive and coding-agent
/// packs require the same closures instead of concatenating chunk-local
/// copies (closes `module.{messages,compaction,branch-summary,system-prompt,
/// agent-session,bash-executor}` and the `modules.chunk-local-helpers` tier).
///
/// Each fragment only assumes a chunk-local `pi` (bound at the top of this
/// chunk); module factories capture it so builtin and file-backed consumers
/// resolve identical closures. This pack is always loaded and non-suppressible.
pub const AGENT_CORE_PACK: EmbeddedPack = EmbeddedPack {
    name: "agent-core",
    source: concat!(
        "local pi = ...\n",
        include_str!("utils/messages.lua"),
        "\n",
        include_str!("utils/branch-summary.lua"),
        "\n",
        include_str!("utils/compaction.lua"),
        "\n",
        include_str!("utils/system-prompt.lua"),
        "\n",
        include_str!("utils/agent-session.lua"),
        "\n",
        include_str!("utils/bash-executor.lua"),
        "\n",
    ),
};

/// The builtin tools pack (spec: `core/tools/`). Loads under the
/// synthetic source key `<tools>`.
/// The Lua-authored default CLI agent pack (spec: coding-agent main loop).
/// The shared agent-policy fragments are owned by the `agent-core` pack
/// (`pi.agent.*` modules); `coding-agent.lua` requires + rebinds them at the
/// top of the file. extensions.lua stays here because it defines the
/// pack-local `EXTENSION_POLICY` / `EXTENSION_CONTEXT_POLICY` tables shared
/// by this pack's policy wiring.
pub const CODING_AGENT_PACK: EmbeddedPack = EmbeddedPack {
    name: "coding-agent",
    source: concat!(
        include_str!("utils/extensions.lua"),
        "\n",
        include_str!("coding-agent.lua"),
    ),
};

pub const TOOLS_PACK: EmbeddedPack = EmbeddedPack {
    name: "tools",
    source: concat!(
        include_str!("tools/prelude.lua"),
        "\n",
        include_str!("utils/syntax-highlight.lua"),
        "\n",
        include_str!("tools/truncate.lua"),
        "\n",
        include_str!("tools/path-utils.lua"),
        "\n",
        include_str!("tools/mime.lua"),
        "\n",
        include_str!("tools/file-mutation-queue.lua"),
        "\n",
        include_str!("tools/shell.lua"),
        "\n",
        include_str!("tools/output-accumulator.lua"),
        "\n",
        include_str!("tools/keybinding-hints.lua"),
        "\n",
        include_str!("tools/visual-truncate.lua"),
        "\n",
        include_str!("tools/render-utils.lua"),
        "\n",
        include_str!("tools/diff.lua"),
        "\n",
        include_str!("tools/read.lua"),
        "\n",
        include_str!("tools/bash.lua"),
        "\n",
        include_str!("tools/edit-diff.lua"),
        "\n",
        include_str!("tools/edit.lua"),
        "\n",
        include_str!("tools/write.lua"),
        "\n",
        include_str!("tools/grep.lua"),
        "\n",
        include_str!("tools/find.lua"),
        "\n",
        include_str!("tools/ls.lua"),
    ),
};
