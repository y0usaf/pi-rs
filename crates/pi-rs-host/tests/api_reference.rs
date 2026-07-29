//! Generated Lua API reference: emit it from the demonstrated surface, then
//! diff it against the committed file.
//!
//! The inventory is never typed by hand. Module members come from walking the
//! live `pi` table an ordinary package receives, and handle methods come from
//! the `impl UserData` blocks in `crates/pi-rs-host/src`. Prose is curated but
//! fail-closed: a member without an entry, or an entry without a member, fails
//! this test, so the reference cannot drift behind the surface.
//!
//! Regenerate with:
//!
//! ```text
//! PI_RS_WRITE_API_REFERENCE=1 cargo test -p pi-rs-host --test api_reference
//! ```

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use pi_rs_host::kernel::{DispatchRequest, RootKind};
use pi_rs_host::{Host, HostConfig, PackageSource};

/// Environment variable that turns the check into a regeneration.
const WRITE_ENV: &str = "PI_RS_WRITE_API_REFERENCE";

/// Committed reference, relative to this crate's manifest directory.
const REFERENCE_PATH: &str = "../../docs/lua-api-reference.md";

/// Ordinary file/embedded package that reports the whole `pi` tree. Tables are
/// walked once: a table reached twice is reported as an alias of the path that
/// reached it first, so `pi.roots.v1.module` cannot be documented twice.
const PROBE: &str = r#"
local pi = ...
local roots = pi.roots.v1

local rows = {}
local seen = {}

local function walk(prefix, value)
  local keys = {}
  for key in pairs(value) do
    if type(key) == "string" then keys[#keys + 1] = key end
  end
  table.sort(keys)
  for _, key in ipairs(keys) do
    local child = value[key]
    local path = prefix .. "." .. key
    local kind = type(child)
    if kind == "table" then
      local origin = seen[child]
      if origin ~= nil then
        rows[#rows + 1] = path .. "\talias\t" .. origin
      else
        seen[child] = path
        rows[#rows + 1] = path .. "\ttable\t"
        walk(path, child)
      end
    elseif kind == "function" then
      rows[#rows + 1] = path .. "\tfunction\t"
    else
      rows[#rows + 1] = path .. "\t" .. kind .. "\t" .. tostring(child)
    end
  end
end

roots.register({
  kind = "application",
  id = "api-reference-probe",
  active = true,
  priority = 0,
  dispatch = function()
    seen[pi] = "pi"
    walk("pi", pi)
    roots.action("reference", { rows = table.concat(rows, "\n") })
  end,
})
"#;

/// Curated prose for every walked module path: `(path, signature, summary)`.
///
/// `signature` is empty for tables, aliases, and constants, and must start with
/// the member's own name for functions.
const MEMBERS: &[(&str, &str, &str)] = &[
    // ---------------------------------------------------------------- auth
    (
        "pi.auth.v1",
        "",
        "Credential storage, resolution, and subscription login at file locations the caller passes in.",
    ),
    (
        "pi.auth.v1.api_version",
        "",
        "Contract version of this module; a breaking change ships as `v2` beside it.",
    ),
    (
        "pi.auth.v1.default_login_timeout_ms",
        "",
        "`options.timeout_ms` used when a login supplies none.",
    ),
    (
        "pi.auth.v1.login",
        "login(provider, callbacks[, options]) -> credentials",
        "Runs one subscription OAuth flow and returns the credential row instead of storing it; every user-visible step is a Lua callback.",
    ),
    (
        "pi.auth.v1.max_login_models",
        "",
        "Most catalog ids `options.model_ids` may enable after a login.",
    ),
    (
        "pi.auth.v1.max_login_timeout_ms",
        "",
        "Largest accepted `options.timeout_ms`.",
    ),
    (
        "pi.auth.v1.max_providers",
        "",
        "Most stored providers one `snapshot()` reports.",
    ),
    (
        "pi.auth.v1.max_secret_bytes",
        "",
        "Largest stored or resolved secret, and the bound on pasted login codes and prompt answers.",
    ),
    (
        "pi.auth.v1.providers",
        "providers() -> rows",
        "Subscription identities that can refresh a stored OAuth row (`id`, `name`, `uses_callback_server`), in registry order.",
    ),
    (
        "pi.auth.v1.store",
        "store{canonical=[, legacy=]} -> CredentialStore",
        "Opens a credential store; `canonical` is the only file ever written and `legacy` is a read-only fallback used only while `canonical` is absent.",
    ),
    // ------------------------------------------------------------- effects
    (
        "pi.effects.v1",
        "",
        "Bounded filesystem, path, environment, process, timer, and cancellation effects.",
    ),
    (
        "pi.effects.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.effects.v1.cancellation",
        "",
        "Standalone abort signals a package can hand to the effects it wants to cancel itself.",
    ),
    (
        "pi.effects.v1.cancellation.new",
        "new() -> AbortSignal",
        "Creates one signal with `abort`, `is_aborted`, and `wait`.",
    ),
    (
        "pi.effects.v1.env",
        "",
        "Immutable snapshot of the process environment taken when the host started; there is no write path.",
    ),
    (
        "pi.effects.v1.env.get",
        "get(name) -> value|nil",
        "Reads one variable from the startup snapshot.",
    ),
    (
        "pi.effects.v1.env.names",
        "names() -> names",
        "Sorted variable names; values never cross in bulk.",
    ),
    (
        "pi.effects.v1.fs",
        "",
        "Bounded filesystem effects; every path is supplied by the caller.",
    ),
    (
        "pi.effects.v1.fs.default_max_bytes",
        "",
        "`read` byte budget when the caller supplies none.",
    ),
    (
        "pi.effects.v1.fs.default_max_entries",
        "",
        "`list` entry budget when the caller supplies none.",
    ),
    (
        "pi.effects.v1.fs.exists",
        "exists(path) -> boolean",
        "Reports presence without raising on a missing path.",
    ),
    (
        "pi.effects.v1.fs.list",
        "list(path[, max_entries]) -> entries",
        "Directory entries, bounded by `max_entries` (`1..=max_entries`).",
    ),
    (
        "pi.effects.v1.fs.make_directory",
        "make_directory(path)",
        "Creates the directory and its missing parents.",
    ),
    (
        "pi.effects.v1.fs.max_bytes",
        "",
        "Largest accepted `read` budget.",
    ),
    (
        "pi.effects.v1.fs.max_entries",
        "",
        "Largest accepted `list` budget.",
    ),
    (
        "pi.effects.v1.fs.read",
        "read(path[, max_bytes]) -> contents",
        "Whole-file read, bounded by `max_bytes`.",
    ),
    (
        "pi.effects.v1.fs.remove_file",
        "remove_file(path)",
        "Removes one file.",
    ),
    (
        "pi.effects.v1.fs.stat",
        "stat(path) -> {type, size, modified_ms}",
        "Path metadata; raises on a missing path, unlike `exists`.",
    ),
    (
        "pi.effects.v1.fs.write",
        "write(path, contents)",
        "Whole-file write.",
    ),
    (
        "pi.effects.v1.path",
        "",
        "Pure POSIX path arithmetic: it computes locations and never chooses one.",
    ),
    (
        "pi.effects.v1.path.basename",
        "basename(path) -> name",
        "Final component of a path.",
    ),
    (
        "pi.effects.v1.path.dirname",
        "dirname(path) -> path",
        "Everything before the final component.",
    ),
    (
        "pi.effects.v1.path.extname",
        "extname(path) -> extension",
        "Extension of the final component, empty when it has none.",
    ),
    (
        "pi.effects.v1.path.is_absolute",
        "is_absolute(path) -> boolean",
        "Whether the path starts at the root.",
    ),
    (
        "pi.effects.v1.path.join",
        "join(...) -> path",
        "Joins components with the separator and normalizes the result.",
    ),
    (
        "pi.effects.v1.path.normalize",
        "normalize(path) -> path",
        "Resolves `.` and `..` textually, without touching the filesystem.",
    ),
    (
        "pi.effects.v1.path.relative",
        "relative(from, to) -> path",
        "Path from one location to another.",
    ),
    (
        "pi.effects.v1.path.resolve",
        "resolve(...) -> path",
        "Joins against the working directory until the result is absolute.",
    ),
    (
        "pi.effects.v1.path.separator",
        "",
        "Component separator used by this module.",
    ),
    ("pi.effects.v1.process", "", "Bounded subprocess execution."),
    (
        "pi.effects.v1.process.default_max_output_bytes",
        "",
        "Captured-output budget when the caller supplies none.",
    ),
    (
        "pi.effects.v1.process.default_timeout_ms",
        "",
        "Run timeout when the caller supplies none.",
    ),
    (
        "pi.effects.v1.process.max_output_bytes",
        "",
        "Largest accepted captured-output budget.",
    ),
    (
        "pi.effects.v1.process.max_timeout_ms",
        "",
        "Largest accepted run timeout.",
    ),
    (
        "pi.effects.v1.process.run",
        "run(program[, args][, options]) -> result",
        "Runs one child process to completion under its timeout and output budget.",
    ),
    ("pi.effects.v1.timer", "", "Scope-owned timers."),
    (
        "pi.effects.v1.timer.sleep",
        "sleep(milliseconds[, signal])",
        "Waits, ending early when the signal aborts; the wait is cancelled and settled before package disposal completes.",
    ),
    // -------------------------------------------------------------- kernel
    (
        "pi.kernel.v1",
        "",
        "The package transaction: roots, declarations, queued actions and effects, modules, immutable handles, and scoped resources.",
    ),
    (
        "pi.kernel.v1.action",
        "action(kind[, payload])",
        "Queues one action; the queue publishes only after the dispatch succeeds.",
    ),
    (
        "pi.kernel.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.kernel.v1.cancellation",
        "cancellation() -> Cancellation",
        "Token of the innermost running dispatch.",
    ),
    (
        "pi.kernel.v1.declare",
        "declare(kind, definition)",
        "The one declaration path for every declared kind, including providers, tools, and themes.",
    ),
    (
        "pi.kernel.v1.effect",
        "effect(kind[, payload])",
        "Queues one effect request alongside the actions of the same dispatch.",
    ),
    (
        "pi.kernel.v1.module",
        "",
        "Exact `name@version` module registry with an explicit reload lifecycle.",
    ),
    (
        "pi.kernel.v1.module.define",
        "define(definition)",
        "Defines one exact identity; a duplicate identity is refused, so there stays one declaration path.",
    ),
    (
        "pi.kernel.v1.module.list",
        "list() -> rows",
        "`name`, `version`, `source`, and `state` (`defined` or `loaded`) in definition order.",
    ),
    (
        "pi.kernel.v1.module.remove",
        "remove(name, version) -> boolean",
        "Drops the declaration and prunes the order index; `remove` then `define` is how a live identity is replaced.",
    ),
    (
        "pi.kernel.v1.module.require",
        "require(name, version) -> value",
        "Resolves one exact identity, running its factory once.",
    ),
    (
        "pi.kernel.v1.module.reset",
        "reset(name, version) -> boolean",
        "Drops only the cached value, so the next `require` re-runs the same factory.",
    ),
    (
        "pi.kernel.v1.read_handle",
        "read_handle(value) -> ReadHandle",
        "Freezes a value behind a generation-stamped read handle.",
    ),
    (
        "pi.kernel.v1.registered",
        "registered(kind) -> definitions",
        "Reads declarations of one kind back in declaration order.",
    ),
    (
        "pi.kernel.v1.resource",
        "resource(disposer) -> Resource",
        "Registers a disposer on the current scope, so cleanup runs at package disposal rather than at Lua garbage collection.",
    ),
    (
        "pi.kernel.v1.root",
        "root(definition)",
        "Registers one root; `pi.roots.v1.register` is the facade over it.",
    ),
    (
        "pi.kernel.v1.roots",
        "roots([kind]) -> rows",
        "Root registrations as data — `kind`, `id`, `source`, `priority`, `active`, `selected` — never the `dispatch` function.",
    ),
    (
        "pi.kernel.v1.select_root",
        "select_root(kind[, id])",
        "Resolves one kind to one registration id regardless of priority; omitting `id` clears the selection.",
    ),
    // -------------------------------------------------------------- models
    (
        "pi.models.v1",
        "",
        "Reviewed-catalog inventory, model-row validation, and bounded provider event streaming.",
    ),
    (
        "pi.models.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.models.v1.apis",
        "apis() -> names",
        "Advertised wire-protocol families a row's `api` may name.",
    ),
    (
        "pi.models.v1.catalog",
        "catalog(provider[, {offset=, limit=}]) -> rows, total",
        "Bounded window of one provider's catalog rows plus the full row count; an unknown provider is an empty window, not an error.",
    ),
    (
        "pi.models.v1.default_max_events",
        "",
        "`options.max_events` used when a stream supplies none.",
    ),
    (
        "pi.models.v1.default_max_models",
        "",
        "`limit` used when a catalog window supplies none.",
    ),
    (
        "pi.models.v1.find",
        "find(provider, id) -> row|nil",
        "One catalog row by provider and id.",
    ),
    (
        "pi.models.v1.max_events",
        "",
        "Largest accepted `options.max_events`.",
    ),
    (
        "pi.models.v1.max_models",
        "",
        "Largest accepted catalog window.",
    ),
    (
        "pi.models.v1.providers",
        "providers() -> names",
        "Every provider name in the reviewed catalog, catalog order.",
    ),
    (
        "pi.models.v1.stream",
        "stream(model, context, options, on_event) -> message",
        "Streams complete provider events and returns the final message; `options.signal` cancels the transport.",
    ),
    (
        "pi.models.v1.validate",
        "validate(row) -> row",
        "Checks a package-authored row against the provider wire schema and returns the canonical row; it stores nothing, and a catalog row validates to itself.",
    ),
    // ------------------------------------------------------------ packages
    (
        "pi.packages.v1",
        "",
        "Bounded package composition: one package loads, lists, and disposes others.",
    ),
    (
        "pi.packages.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.packages.v1.list",
        "list() -> rows",
        "Still-loaded packages in load order (`source`, `scope`, `owner`).",
    ),
    (
        "pi.packages.v1.load",
        "load{path=} | load{name=, source=} -> Package",
        "Loads one package through the same provenance path the host uses, as a disposable resource of the loader.",
    ),
    (
        "pi.packages.v1.max_depth",
        "",
        "Deepest chain of nested loads.",
    ),
    (
        "pi.packages.v1.max_packages",
        "",
        "Most Lua-loaded packages alive at once.",
    ),
    (
        "pi.packages.v1.max_source_bytes",
        "",
        "Largest accepted package source.",
    ),
    // ------------------------------------------------------------- records
    (
        "pi.records.v1",
        "",
        "Durable append-only record stores at destinations the caller passes in; the schema of a record is never interpreted.",
    ),
    (
        "pi.records.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.records.v1.create",
        "create{directory=, name=[, limits=][, cancellation=]} -> RecordStore",
        "Creates a store and takes its exclusive file lock.",
    ),
    (
        "pi.records.v1.default_limits",
        "",
        "Window and record bounds applied when a call supplies no `limits`.",
    ),
    (
        "pi.records.v1.default_limits.max_record_bytes",
        "",
        "Largest single appended record.",
    ),
    (
        "pi.records.v1.default_limits.max_window_bytes",
        "",
        "Largest encoded byte count one cursor window returns.",
    ),
    (
        "pi.records.v1.default_limits.max_window_records",
        "",
        "Most records one cursor window returns.",
    ),
    (
        "pi.records.v1.extension",
        "",
        "File extension of a store on disk.",
    ),
    (
        "pi.records.v1.format_version",
        "",
        "On-disk format version written into every store header.",
    ),
    (
        "pi.records.v1.list",
        "list{directory=[, limits=][, cancellation=]} -> {stores, diagnostics}",
        "Lists stores in a directory and reports locked or damaged files as explicit diagnostics rather than omitting them.",
    ),
    (
        "pi.records.v1.open",
        "open{path=[, limits=][, cancellation=]} -> RecordStore",
        "Opens an existing store and takes its exclusive file lock.",
    ),
    // --------------------------------------------------------------- roots
    (
        "pi.roots.v1",
        "",
        "Facade over the kernel transaction for application, agent, and frontend roots.",
    ),
    (
        "pi.roots.v1.action",
        "action(kind[, payload])",
        "The same queue as `pi.kernel.v1.action`.",
    ),
    (
        "pi.roots.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.roots.v1.cancellation",
        "cancellation() -> Cancellation",
        "The same dispatch token as `pi.kernel.v1.cancellation`.",
    ),
    (
        "pi.roots.v1.list",
        "list([kind]) -> rows",
        "The same registry rows as `pi.kernel.v1.roots`, restricted to the kinds this facade registers.",
    ),
    (
        "pi.roots.v1.dispatch",
        "dispatch(kind, event[, context]) -> batch",
        "Runs one root dispatch from Lua and returns its published `generation`, `source`, `actions`, and `effects`.",
    ),
    (
        "pi.roots.v1.middleware",
        "",
        "Root middleware registration.",
    ),
    (
        "pi.roots.v1.middleware.register",
        "register(definition)",
        "Registers an `event` or `render` stage for one root kind, ordered by `order` then registration.",
    ),
    (
        "pi.roots.v1.module",
        "",
        "The same module registry the kernel exposes.",
    ),
    (
        "pi.roots.v1.register",
        "register(definition)",
        "Registers one `application`, `agent`, or `frontend` root with its `id`, `dispatch`, `active`, and `priority`.",
    ),
    (
        "pi.roots.v1.select",
        "select(kind[, id])",
        "Names the registration one kind resolves to, outranking priority; the selection is owned by the selecting source and scope.",
    ),
    // ------------------------------------------------------------ terminal
    (
        "pi.terminal.v1",
        "",
        "Batched terminal input, the retained display, and the Unicode cell measurement a package lays out with.",
    ),
    (
        "pi.terminal.v1.api_version",
        "",
        "Contract version of this module.",
    ),
    (
        "pi.terminal.v1.display",
        "display([limits]) -> Display",
        "Retained display handle; `limits` may set `max_link_bytes`, `max_images`, and `max_image_bytes`.",
    ),
    (
        "pi.terminal.v1.display_schema_version",
        "",
        "Schema version every submitted batch must carry.",
    ),
    (
        "pi.terminal.v1.input_buffer",
        "input_buffer() -> InputBuffer",
        "Bounded input decoder; no callback occurs per byte.",
    ),
    (
        "pi.terminal.v1.text",
        "",
        "Cell arithmetic walked with the same traversal the rasterizer paints with, so a measured string and the painted node agree by construction.",
    ),
    (
        "pi.terminal.v1.text.default_max_graphemes",
        "",
        "`graphemes` window size when the caller supplies none.",
    ),
    (
        "pi.terminal.v1.text.default_max_rows",
        "",
        "`wrap` row budget when the caller supplies none.",
    ),
    (
        "pi.terminal.v1.text.graphemes",
        "graphemes(text[, {offset=, limit=}]) -> clusters, total",
        "Bounded window of `{byte, width, text}` clusters plus the total cluster count; `byte` is one-based.",
    ),
    (
        "pi.terminal.v1.text.max_bytes",
        "",
        "Largest input any member of this table accepts.",
    ),
    (
        "pi.terminal.v1.text.max_graphemes",
        "",
        "Largest accepted `graphemes` window.",
    ),
    (
        "pi.terminal.v1.text.max_rows",
        "",
        "Largest accepted `wrap` row budget.",
    ),
    (
        "pi.terminal.v1.text.measure",
        "measure(text, {width=[, wrap=][, tab_width=]}) -> {rows, max_width, last_width, cells}",
        "What that text occupies in a node `width` cells wide; a node sized from `measure` paints exactly `cells`.",
    ),
    (
        "pi.terminal.v1.text.truncate",
        "truncate(text, {width=[, ellipsis=]}) -> text, width, truncated",
        "Shortens to a budget without splitting a grapheme or half-painting a wide cluster.",
    ),
    (
        "pi.terminal.v1.text.width",
        "width(text) -> cells",
        "Cell width of one single-line string; a newline or tab is refused because both change layout rather than width.",
    ),
    (
        "pi.terminal.v1.text.wrap",
        "wrap(text, {width=[, tab_width=][, limit=]}) -> rows, overflowed",
        "The row strings that node would paint, plus whether rows past `limit` were dropped.",
    ),
];

/// Curated prose for each handle type: `(rust type, documented name, origin)`.
const HANDLES: &[(&str, &str, &str)] = &[
    (
        "LuaRetainedDisplay",
        "Display",
        "Returned by `pi.terminal.v1.display([limits])`.",
    ),
    (
        "LuaStdinBuffer",
        "InputBuffer",
        "Returned by `pi.terminal.v1.input_buffer()`.",
    ),
    (
        "LuaStore",
        "RecordStore",
        "Returned by `pi.records.v1.create` and `pi.records.v1.open`; every open store is a scope resource of its package.",
    ),
    (
        "LuaCursor",
        "RecordCursor",
        "Returned by `RecordStore:cursor()`.",
    ),
    (
        "LuaPackage",
        "Package",
        "Returned by `pi.packages.v1.load`.",
    ),
    (
        "LuaCredentialStore",
        "CredentialStore",
        "Returned by `pi.auth.v1.store`; it owns no operating-system resource between calls.",
    ),
    (
        "LuaCancellation",
        "Cancellation",
        "Returned by `pi.kernel.v1.cancellation()` and carried by every dispatch snapshot.",
    ),
    (
        "LuaReadHandle",
        "ReadHandle",
        "Returned by `pi.kernel.v1.read_handle(value)`.",
    ),
    (
        "LuaResource",
        "Resource",
        "Returned by `pi.kernel.v1.resource(disposer)`.",
    ),
    (
        "LuaAbortSignal",
        "AbortSignal",
        "Returned by `pi.effects.v1.cancellation.new()` and accepted by the effects that can be cancelled.",
    ),
];

/// Curated prose per handle method: `("Handle.method", signature, summary)`.
const HANDLE_MEMBERS: &[(&str, &str, &str)] = &[
    (
        "Display.submit",
        "submit(batch) -> {revision, painted_cells, placed_images}",
        "Submits one complete versioned tree and presents the difference; nothing crosses per cell.",
    ),
    (
        "Display.revision",
        "revision() -> number",
        "Revision of the retained tree, unchanged when a submission changes nothing.",
    ),
    (
        "Display.reset_presentation",
        "reset_presentation()",
        "Forgets what the terminal is believed to show, so the next submit repaints everything.",
    ),
    (
        "InputBuffer.feed",
        "feed(data) -> events",
        "Decodes one chunk into complete input events.",
    ),
    (
        "InputBuffer.flush",
        "flush() -> events",
        "Decodes whatever is buffered, resolving a pending ambiguous prefix.",
    ),
    ("InputBuffer.clear", "clear()", "Drops buffered bytes."),
    (
        "InputBuffer.buffer",
        "buffer() -> data",
        "The undecoded bytes still held.",
    ),
    (
        "RecordStore.path",
        "path() -> path",
        "On-disk location of this store.",
    ),
    (
        "RecordStore.record_count",
        "record_count() -> number",
        "Records appended so far.",
    ),
    (
        "RecordStore.append",
        "append(value[, options]) -> sequence",
        "Appends one opaque record and returns its sequence number.",
    ),
    (
        "RecordStore.cursor",
        "cursor() -> RecordCursor",
        "Opens a bounded reader over this store.",
    ),
    (
        "RecordStore.copy",
        "copy{directory=, name=[, record_count=]} -> path",
        "Atomic prefix snapshot of this store at another destination.",
    ),
    (
        "RecordStore.close",
        "close()",
        "Closes the file and releases the lock without waiting for disposal.",
    ),
    (
        "RecordStore.closed",
        "closed() -> boolean",
        "Whether this store has already been closed.",
    ),
    (
        "RecordCursor.next_sequence",
        "next_sequence() -> number",
        "Sequence this cursor will read next.",
    ),
    (
        "RecordCursor.next",
        "next([{max_records=, max_bytes=, cancellation=}]) -> window",
        "One bounded window carrying `records`, `start_sequence`, `next_sequence`, `encoded_bytes`, and `done`.",
    ),
    (
        "Package.source",
        "source() -> source",
        "Provenance string of the loaded package.",
    ),
    (
        "Package.scope",
        "scope() -> number",
        "Scope that owns the loaded package's registrations.",
    ),
    (
        "Package.dispose",
        "dispose()",
        "Disposes the loaded package and, transitively, everything it composed.",
    ),
    (
        "Package.disposed",
        "disposed() -> boolean",
        "Whether this package has already been disposed.",
    ),
    (
        "CredentialStore.snapshot",
        "snapshot() -> {source, providers}",
        "Provenance (`canonical`, `legacy`, or `absent`) and stored provider names; never a secret.",
    ),
    (
        "CredentialStore.describe",
        "describe(provider) -> description|nil",
        "`kind` plus, for an OAuth row, `expires`, `expired`, and the provider-defined `extra_fields` names; never a secret.",
    ),
    (
        "CredentialStore.set_api_key",
        "set_api_key(provider, value)",
        "Stores an api-key expression; the first write promotes storage to `canonical` and migrates legacy rows forward.",
    ),
    (
        "CredentialStore.set_oauth",
        "set_oauth(provider, credentials)",
        "Stores an OAuth row, preserving every provider-defined extra field verbatim.",
    ),
    (
        "CredentialStore.remove",
        "remove(provider)",
        "Removes one stored row.",
    ),
    (
        "CredentialStore.resolve",
        "resolve(provider) -> {api_key, refreshed}|nil",
        "The only member that yields a secret: it expands a stored expression and refreshes an expired OAuth row under the same lock.",
    ),
    (
        "Cancellation.is_cancelled",
        "is_cancelled() -> boolean",
        "Whether the owning dispatch has been cancelled.",
    ),
    (
        "Cancellation.wait",
        "wait()",
        "Waits until the owning dispatch is cancelled.",
    ),
    (
        "ReadHandle.generation",
        "generation() -> number",
        "Generation the handle was issued in.",
    ),
    (
        "ReadHandle.read",
        "read() -> value",
        "Reads the frozen value back.",
    ),
    (
        "Resource.dispose",
        "dispose()",
        "Runs the disposer now instead of at scope teardown.",
    ),
    (
        "Resource.disposed",
        "disposed() -> boolean",
        "Whether the disposer has already run.",
    ),
    (
        "AbortSignal.is_aborted",
        "is_aborted() -> boolean",
        "Whether this signal has been aborted.",
    ),
    ("AbortSignal.abort", "abort()", "Aborts this signal."),
    (
        "AbortSignal.wait",
        "wait()",
        "Waits until this signal is aborted.",
    ),
];

/// mlua registration calls this scan understands. An unrecognized `add_*` call
/// inside an `impl UserData` block fails the test rather than being skipped.
const REGISTRATION_CALLS: &[&str] = &[
    "add_method",
    "add_method_mut",
    "add_async_method",
    "add_async_method_mut",
    "add_function",
    "add_function_mut",
    "add_async_function",
    "add_meta_method",
    "add_meta_method_mut",
    "add_meta_function",
    "add_field",
    "add_field_method_get",
    "add_field_method_set",
    "add_fields",
    "add_methods",
];

/// One walked entry: its Lua kind and, for constants and aliases, its detail.
type Walked = BTreeMap<String, (String, String)>;

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Walks the `pi` table an ordinary package receives.
fn walk_surface() -> Walked {
    let host = Host::new(HostConfig::default()).expect("host starts");
    host.load_package(PackageSource::Embedded {
        name: "api-reference-probe",
        source: PROBE,
    })
    .expect("probe package loads");
    let batch = host
        .dispatch(DispatchRequest::new(
            RootKind::Application,
            serde_json::json!({ "kind": "reference" }),
            serde_json::json!({}),
        ))
        .expect("probe dispatch");
    let rows = batch.actions[0].payload["rows"]
        .as_str()
        .expect("probe reports rows")
        .to_owned();

    let mut walked = Walked::new();
    for row in rows.lines() {
        let mut fields = row.splitn(3, '\t');
        let path = fields.next().expect("path field").to_owned();
        let kind = fields.next().expect("kind field").to_owned();
        let detail = fields.next().unwrap_or_default().to_owned();
        assert!(
            walked.insert(path.clone(), (kind, detail)).is_none(),
            "the probe reported `{path}` twice"
        );
    }
    assert!(!walked.is_empty(), "the probe reported an empty surface");
    walked
}

fn collect_rust_sources(directory: &Path, into: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory).expect("read host source directory");
    for entry in entries {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, into);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            into.push(path);
        }
    }
}

/// Reads the method names out of one `impl UserData` block. Fails loudly on any
/// registration form it cannot read, so a new form cannot silently escape the
/// reference.
fn registered_methods(block: &str, type_name: &str, file: &Path) -> Vec<String> {
    let bytes = block.as_bytes();
    let mut names = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = block[cursor..].find(".add_") {
        let start = cursor + offset + 1;
        let mut end = start;
        while end < bytes.len() && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_') {
            end += 1;
        }
        let call = &block[start..end];
        assert!(
            REGISTRATION_CALLS.contains(&call),
            "unrecognized userdata registration `{call}` on `{type_name}` in {}; \
             teach crates/pi-rs-host/tests/api_reference.rs about it",
            file.display()
        );
        cursor = end;
        // `add_methods`/`add_fields` are the trait entry points, not members.
        if call == "add_methods" || call == "add_fields" {
            continue;
        }
        let mut probe = end;
        while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
            probe += 1;
        }
        assert_eq!(
            bytes.get(probe),
            Some(&b'('),
            "`{call}` on `{type_name}` in {} is not a direct call",
            file.display()
        );
        probe += 1;
        while probe < bytes.len() && bytes[probe].is_ascii_whitespace() {
            probe += 1;
        }
        assert_eq!(
            bytes.get(probe),
            Some(&b'"'),
            "`{call}` on `{type_name}` in {} does not name its member with a string literal; \
             the reference can only be generated from literal names",
            file.display()
        );
        probe += 1;
        let name_start = probe;
        while probe < bytes.len() && bytes[probe] != b'"' {
            probe += 1;
        }
        names.push(block[name_start..probe].to_owned());
        cursor = probe;
    }
    names
}

/// Every `impl UserData` type in the host with at least one member, in source
/// order per file. `cargo fmt` guarantees a top-level block closes with `}` in
/// the first column, which is what bounds the scan.
fn scan_handles() -> BTreeMap<String, Vec<String>> {
    let mut files = Vec::new();
    collect_rust_sources(&crate_dir().join("src"), &mut files);
    files.sort();

    let mut handles: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for file in &files {
        let text = std::fs::read_to_string(file).expect("read host source file");
        let lines: Vec<&str> = text.lines().collect();
        let mut index = 0usize;
        while index < lines.len() {
            let Some(rest) = lines[index].strip_prefix("impl UserData for ") else {
                index += 1;
                continue;
            };
            let type_name = rest.split('{').next().expect("type name").trim().to_owned();
            if lines[index].contains("{}") {
                index += 1;
                continue;
            }
            let mut end = index + 1;
            while end < lines.len() && lines[end] != "}" {
                end += 1;
            }
            assert!(
                end < lines.len(),
                "unterminated `impl UserData for {type_name}` in {}",
                file.display()
            );
            let block = lines[index + 1..end]
                .iter()
                .filter(|line| !line.trim_start().starts_with("//"))
                .copied()
                .collect::<Vec<_>>()
                .join("\n");
            let methods = registered_methods(&block, &type_name, file);
            if !methods.is_empty() {
                assert!(
                    handles.insert(type_name.clone(), methods).is_none(),
                    "`{type_name}` has more than one `impl UserData` block"
                );
            }
            index = end + 1;
        }
    }
    assert!(
        !handles.is_empty(),
        "the handle scan found no `impl UserData` blocks; the scan is broken, not the surface"
    );
    handles
}

fn parent_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(head, _)| head).unwrap_or("")
}

fn leaf_of(path: &str) -> &str {
    path.rsplit_once('.').map(|(_, tail)| tail).unwrap_or(path)
}

fn depth_of(path: &str) -> usize {
    path.split('.').count()
}

/// Whether a curated signature opens with the generated member name. A call
/// taking a table opens with a brace (`store{canonical=...}`), everything else
/// with a paren.
fn opens_with_name(signature: &str, name: &str) -> bool {
    signature
        .strip_prefix(name)
        .is_some_and(|rest| rest.starts_with('(') || rest.starts_with('{'))
}

/// Escapes the one character a Markdown table cell cannot carry literally.
/// Signatures such as `find(provider, id) -> row|nil` would otherwise open a
/// fourth column.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Renders one member row. The name and kind come from the walk; only the
/// sentence is curated.
fn member_row(
    path: &str,
    kind: &str,
    detail: &str,
    prose: &BTreeMap<&str, (&str, &str)>,
) -> String {
    let (signature, summary) = prose[path];
    let name = leaf_of(path);
    let kind_cell = match kind {
        "function" => "function".to_owned(),
        "table" => "table".to_owned(),
        "alias" => format!("alias of `{detail}`"),
        "string" => format!("`\"{detail}\"`"),
        _ => format!("`{detail}`"),
    };
    let summary_cell = if signature.is_empty() {
        cell(summary)
    } else {
        format!("`{}` — {}", cell(signature), cell(summary))
    };
    format!("| `{name}` | {kind_cell} | {summary_cell} |")
}

fn render_container(
    container: &str,
    walked: &Walked,
    prose: &BTreeMap<&str, (&str, &str)>,
    out: &mut String,
) {
    let children: Vec<&String> = walked
        .keys()
        .filter(|path| parent_of(path) == container)
        .collect();
    if children.is_empty() {
        return;
    }
    out.push_str("| Member | Kind | Summary |\n| --- | --- | --- |\n");
    for path in &children {
        let (kind, detail) = &walked[*path];
        out.push_str(&member_row(path, kind, detail, prose));
        out.push('\n');
    }
    out.push('\n');
    for path in &children {
        if walked[*path].0 != "table" {
            continue;
        }
        let level = "#".repeat((depth_of(path)).min(6));
        out.push_str(&format!("{level} `{path}`\n\n"));
        render_container(path, walked, prose, out);
    }
}

fn render(walked: &Walked, handles: &BTreeMap<String, Vec<String>>) -> String {
    let prose: BTreeMap<&str, (&str, &str)> = MEMBERS
        .iter()
        .map(|(path, signature, summary)| (*path, (*signature, *summary)))
        .collect();

    let handle_prose: BTreeMap<&str, (&str, &str)> = HANDLE_MEMBERS
        .iter()
        .map(|(key, signature, summary)| (*key, (*signature, *summary)))
        .collect();

    let modules: Vec<&String> = walked
        .keys()
        .filter(|path| depth_of(path) == 3 && walked[*path].0 == "table")
        .collect();
    let member_count = walked.values().filter(|(kind, _)| kind != "table").count();
    let method_count: usize = handles.values().map(Vec::len).sum();

    let mut out = String::new();
    out.push_str("# Lua API reference\n\n");
    out.push_str(
        "<!-- Generated by crates/pi-rs-host/tests/api_reference.rs. Do not edit by hand. -->\n\
         <!-- Regenerate: PI_RS_WRITE_API_REFERENCE=1 cargo test -p pi-rs-host --test api_reference -->\n\n",
    );
    out.push_str(
        "Every member below was walked out of the live `pi` table an ordinary package\n\
         receives, and every handle method was read out of its `impl UserData` block in\n\
         `crates/pi-rs-host/src`. Names, kinds, and constant values are generated; only the\n\
         sentences are written by hand, and a member with no sentence — or a sentence with\n\
         no member — fails `cargo test -p pi-rs-host --test api_reference`. Embedded and\n\
         file-backed packages receive the same table, so nothing here is provenance-specific.\n\n",
    );
    out.push_str(
        "This is the inventory. `docs/lua-extension-api.md` explains the mechanisms and\n\
         their rules; `docs/lua-coding-spine.md` explains the coding-spine event and display\n\
         contracts.\n\n",
    );
    out.push_str(&format!(
        "Surface: {} modules, {member_count} members, {} handles, {method_count} handle methods.\n\n",
        modules.len(),
        handles.len()
    ));

    out.push_str("## Modules\n\n");
    out.push_str(
        "Each top-level namespace holds exactly one version table, so a breaking change\n\
         ships beside its predecessor rather than replacing it.\n\n",
    );
    for path in &modules {
        out.push_str(&format!("### `{path}`\n\n"));
        let (_, summary) = prose[path.as_str()];
        out.push_str(summary);
        out.push_str("\n\n");
        render_container(path, walked, &prose, &mut out);
    }

    out.push_str("## Handles\n\n");
    out.push_str(
        "Handles are values the modules above return. They are not reachable from the `pi`\n\
         table and carry no members beyond these. Sections follow the reading order of the\n\
         modules that hand them out; methods follow their `impl UserData` block.\n\n",
    );
    for (rust_type, name, origin) in HANDLES {
        let methods = &handles[*rust_type];
        out.push_str(&format!("### `{name}`\n\n{origin}\n\n"));
        out.push_str("| Method | Summary |\n| --- | --- |\n");
        for method in methods {
            let key = format!("{name}.{method}");
            let (signature, summary) = handle_prose[key.as_str()];
            out.push_str(&format!(
                "| `{method}` | `{}` — {} |\n",
                cell(signature),
                cell(summary)
            ));
        }
        out.push('\n');
    }
    // One trailing newline, so the committed file has no blank line at EOF.
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

fn report_difference(label: &str, expected: &BTreeSet<String>, actual: &BTreeSet<String>) {
    let missing: Vec<&String> = expected.difference(actual).collect();
    let extra: Vec<&String> = actual.difference(expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{label} drifted.\n  documented but absent from the surface: {missing:?}\n  \
         present in the surface but undocumented: {extra:?}\n  \
         Update crates/pi-rs-host/tests/api_reference.rs, then regenerate with \
         PI_RS_WRITE_API_REFERENCE=1 cargo test -p pi-rs-host --test api_reference"
    );
}

#[test]
fn the_generated_reference_matches_the_demonstrated_surface() {
    let walked = walk_surface();
    let handles = scan_handles();

    // Versioning invariant: a namespace holds exactly one version table, and
    // nothing else. This is why the curated prose starts at `pi.<name>.v1`.
    for (path, (kind, _)) in &walked {
        match depth_of(path) {
            2 => assert_eq!(kind, "table", "`{path}` must be a version namespace"),
            3 => assert_eq!(
                leaf_of(path),
                "v1",
                "`{path}` is not a version table; the reference assumes `pi.<name>.v1`"
            ),
            _ => {}
        }
    }

    let documented: BTreeSet<String> = MEMBERS
        .iter()
        .map(|(path, _, _)| (*path).to_owned())
        .collect();
    assert_eq!(
        documented.len(),
        MEMBERS.len(),
        "a path is listed twice in MEMBERS"
    );
    let present: BTreeSet<String> = walked
        .keys()
        .filter(|path| depth_of(path) > 2)
        .cloned()
        .collect();
    report_difference("The module surface", &documented, &present);

    // A function's curated signature must open with its own generated name, so
    // the prose cannot describe a member the walk did not find. A table-argument
    // call (`store{canonical=...}`) opens with a brace instead of a paren.
    for (path, signature, _) in MEMBERS {
        let (kind, _) = &walked[*path];
        if kind == "function" {
            assert!(
                opens_with_name(signature, leaf_of(path)),
                "signature for `{path}` must open with `{}(` or `{}{{`",
                leaf_of(path),
                leaf_of(path)
            );
        } else {
            assert!(
                signature.is_empty(),
                "`{path}` is a {kind}, so it carries no signature"
            );
        }
    }

    // Handle types and their methods, both directions.
    let documented_handles: BTreeSet<String> = HANDLES
        .iter()
        .map(|(rust, _, _)| (*rust).to_owned())
        .collect();
    let scanned_handles: BTreeSet<String> = handles.keys().cloned().collect();
    report_difference(
        "The handle inventory",
        &documented_handles,
        &scanned_handles,
    );

    let handle_names: BTreeMap<&str, &str> = HANDLES
        .iter()
        .map(|(rust, name, _)| (*rust, *name))
        .collect();
    let scanned_members: BTreeSet<String> = handles
        .iter()
        .flat_map(|(rust_type, methods)| {
            let name = handle_names[rust_type.as_str()];
            methods.iter().map(move |method| format!("{name}.{method}"))
        })
        .collect();
    let documented_members: BTreeSet<String> = HANDLE_MEMBERS
        .iter()
        .map(|(key, _, _)| (*key).to_owned())
        .collect();
    report_difference("The handle methods", &documented_members, &scanned_members);
    for (key, signature, _) in HANDLE_MEMBERS {
        assert!(
            opens_with_name(signature, leaf_of(key)),
            "signature for `{key}` must open with `{}(` or `{}{{`",
            leaf_of(key),
            leaf_of(key)
        );
    }

    let rendered = render(&walked, &handles);
    let reference = crate_dir().join(REFERENCE_PATH);
    if std::env::var_os(WRITE_ENV).is_some() {
        std::fs::write(&reference, &rendered).expect("write the generated reference");
        return;
    }
    let committed = std::fs::read_to_string(&reference).unwrap_or_default();
    if committed == rendered {
        return;
    }
    let first_difference = rendered
        .lines()
        .zip(committed.lines())
        .enumerate()
        .find(|(_, (left, right))| left != right)
        .map(|(index, (left, right))| {
            format!(
                "line {}:\n  generated: {left}\n  committed: {right}",
                index + 1
            )
        })
        .unwrap_or_else(|| {
            format!(
                "length: generated {} lines, committed {} lines",
                rendered.lines().count(),
                committed.lines().count()
            )
        });
    panic!(
        "docs/lua-api-reference.md is stale.\n{first_difference}\n\
         Regenerate with: PI_RS_WRITE_API_REFERENCE=1 cargo test -p pi-rs-host --test api_reference"
    );
}
