# Lua mechanism API

The inherited Pi-compatibility extension API is not part of pi-rs. Ordinary Lua
packages receive one compact, versioned mechanism table; embedded provenance
adds no members or privileges.

The complete top-level surface after PLAN 4.1's credential-storage slice is:

- `pi.kernel.v1`: package transaction primitives;
- `pi.roots.v1`: application/agent/frontend root facade;
- `pi.terminal.v1`: batched terminal input + retained display submission;
- `pi.models.v1`: catalog inventory/lookup, model-row validation, and bounded
  provider event streaming;
- `pi.effects.v1`: bounded filesystem/path/environment, process, timer, and
  cancellation effects;
- `pi.records.v1`: durable append-only record stores at Lua-chosen destinations;
- `pi.packages.v1`: bounded package composition, listing, and disposal;
- `pi.auth.v1`: credential storage and resolution at Lua-chosen locations, plus
  the subscription-provider inventory.

No top-level event bus, command/tool registry, runtime/session/config/settings,
trust/login UI, `pi.ai`, `pi.tui`, `pi.fs`, `pi.exec`, or `pi.http` compatibility
member is installed. Product declarations and richer capabilities return only
with a demonstrated file-backed consumer.

## Package transaction

A package is a Lua chunk receiving `pi` as `...`. File, memory, and embedded
sources use the same loader, watchdog, publication transaction, scope ownership,
and disposal path.

`pi.kernel.v1` provides:

- `root(definition)`, `declare(kind, definition)`, and `registered(kind)`;
- `action(kind, payload)` and `effect(kind, payload)` queues;
- exact-version `module.define`, `module.require`, `module.list`,
  `module.remove`, and `module.reset`;
- immutable `read_handle(value)`, dispatch `cancellation()`, and scoped
  `resource(disposer)`.

Actions/effects publish only after a successful dispatch. Snapshots contain
`version`, `generation`, `scope`, immutable `event`, and immutable `context`.

`pi.roots.v1.register(definition)` accepts `kind = "application" | "agent" |
"frontend"`; `action`, `cancellation`, and `module` are the same canonical
kernel operations.

## Module lifecycle

`pi.kernel.v1.module` resolves exact `name@version` identities. `define` still
refuses a duplicate identity, so there is one declaration path; reload is an
explicit lifecycle operation on that declaration:

- `remove(name, version)` drops the declaration and returns `true`, or `false`
  when nothing was defined. The order index is pruned, so redefining the same
  identity is still listed once, in its new position;
- `reset(name, version)` drops only the cached value and returns `true` when
  there was one, so the next `require` re-runs the same factory with the same
  dependency aliases;
- `list()` reports `name`, `version`, `source`, and `state` (`"defined"` or
  `"loaded"`) in definition order.

Both are scope-local: a package may reload only what it defined, and a sibling
gets an error naming the owning source. A module whose factory is running is
refused, because its dependents are mid-resolution. Rust invalidates nothing
else — a dependent that already cached a value keeps it until its owner resets
it, so reload order is Lua policy, exactly like package generation swaps.

A module value is an ordinary Lua value with no cleanup hook. A module that owns
something disposable registers it through `pi.kernel.v1.resource` and exposes
the handle, so its owner disposes it before reloading; package disposal remains
the other lifecycle path and removes that package's modules with its scope.

## Package composition

`pi.packages.v1` lets one package compose others. Rust chooses no location,
order, name, or reload policy: the caller passes an explicit request, so
discovery, precedence, and swap order stay in Lua.

- `load{path=...}` or `load{name=..., source=...}` resolves bytes through the
  same provenance path as the host package API and returns a handle with
  `source()`, `scope()`, `dispose()`, and `disposed()`;
- `list()` returns the still-loaded packages in load order (`source`, `scope`,
  `owner`);
- `api_version`, `max_depth` (4 nested loads), `max_packages` (64 at once), and
  `max_source_bytes` (4 MiB) describe the bounds.

A loaded package is one disposable resource of its loader, registered through
the same path as `pi.kernel.v1.resource`: disposing the composing package
disposes everything it composed, transitively, and runs each composed package's
own disposers. A package may not dispose the package currently running or one
whose load has not finished, and the same source cannot be loaded twice at once.

A nested load inherits the caller's watchdog budget, so composition stays inside
the one bounded dispatch. The caller's publication queue is stacked for the
duration: a loaded chunk cannot append to the loading dispatch's batch. Atomic
replacement is Lua policy — load the new generation, then dispose the old — so a
failed load leaves the previous generation selected.

## Terminal

`pi.terminal.v1.input_buffer()` returns a bounded decoder with `feed`, `flush`,
`clear`, and `buffer`. `display([limits])` returns a retained display handle with
`submit`, `revision`, and `reset_presentation`. Submit complete versioned trees
using `display_schema_version`; no callback occurs per byte or cell.

## Models

`pi.models.v1.find(provider, id)` returns a catalog model row or `nil`.
`stream(model, context, options, on_event)` streams complete provider events and
returns the final message. `options.max_events` defaults to `default_max_events`
(256) and is limited to `1..=max_events` (1024); transport cancellation uses
`options.signal`.

Inventory is mechanism data, so a package can present or validate providers
without streaming first:

- `providers()` — every provider name in the reviewed catalog, catalog order;
- `catalog(provider[, {offset=, limit=}])` — a bounded window of that
  provider's model rows plus the full row count as a second return value;
  `limit` defaults to `default_max_models` (64) and is limited to
  `1..=max_models` (512). An unknown provider is an empty window, not an error;
- `apis()` — the advertised wire-protocol families a row's `api` may name;
- `validate(row)` — check a package-authored row against the provider wire
  schema and return the canonical row. It refuses an unregistered `api`
  (naming the supported families) and an empty `id`, `provider`, or `baseUrl`.
  A catalog row validates to itself, so custom endpoints and catalog rows are
  the same kind of value.

`validate` stores nothing. Provider declarations use the one generic
declaration path, `pi.kernel.v1.declare("provider", definition)`, and
`registered("provider")` reads them back in the declaring package's chosen
order. Which providers exist, their endpoints, their ordering, and which one a
dispatch streams through are Lua policy; Rust only says which rows the reviewed
catalog holds and which wire protocols it can dispatch.

## Effects

`pi.effects.v1.fs.read(path[, max_bytes])` and `write(path, contents)` are capped
at 8 MiB. `exists(path)`, `stat(path)` (`type`, `size`, `modified_ms`),
`list(path[, max_entries])`, `make_directory(path)` (recursive), and
`remove_file(path)` cover path metadata; listings default to 1024 entries and
are limited to `1..=16384` (`default_max_entries`, `max_entries`). `stat` on a
missing path raises; `exists` does not.

`pi.effects.v1.path` is pure POSIX path arithmetic — `join`, `normalize`,
`dirname`, `basename`, `extname`, `is_absolute`, `resolve`, `relative`, and
`separator`. It computes locations; it never chooses one.

`pi.effects.v1.env` is an immutable snapshot of the process environment taken
when the host starts: `get(name)` returns one value or `nil`, and `names()`
returns the sorted variable names. Values never cross in bulk, the snapshot
cannot be written, and no variable has a meaning in Rust — XDG/legacy
precedence, credential variables, and defaults are Lua policy. Embedders may
supply an explicit environment through `HostConfig::environment`.

`process.run(program[, args][, options])` caps timeout at five minutes
and output at 8 MiB. `timer.sleep(milliseconds[, signal])` is scope-owned.
`cancellation.new()` returns a signal with `abort`, `is_aborted`, and `wait`.
All queued work is cancelled and settled before package disposal completes.

## Records

`pi.records.v1` persists opaque JSON records; it never interprets a schema and
never chooses a path. Callers pass explicit destinations, normally derived from
the immutable `snapshot.context` storage data, so XDG/legacy policy stays in
Lua.

- `create{directory, name[, limits][, cancellation]}` and
  `open{path[, limits][, cancellation]}` return a store holding an exclusive
  file lock;
- `list{directory[, limits][, cancellation]}` returns `stores` plus explicit
  `diagnostics` (`locked`, `header`, `corruption`, `partial-write`, `io`), so a
  locked or damaged file is reported rather than silently omitted;
- `api_version`, `format_version`, `extension`, and `default_limits`
  (`max_record_bytes` 1 MiB, `max_window_records` 256, `max_window_bytes`
  4 MiB) describe the mechanism.

A store provides `path`, `record_count`, `append(value[, options])` returning
the record sequence, `cursor()`, `copy{directory, name[, record_count]}` for an
atomic prefix snapshot, `close()`, and `closed()`. A cursor provides
`next_sequence()` and `next([{max_records, max_bytes, cancellation}])`, whose
window carries `records`, `start_sequence`, `next_sequence`, `encoded_bytes`,
and `done`. Windows are bounded by the store limits, so iteration never copies
an unbounded history.

Every open store is registered on the owning package or dispatch scope through
the same path as `pi.kernel.v1.resource`: disposing the package closes the store
and releases its lock without waiting for Lua garbage collection. Operations are
synchronous and observe the innermost dispatch cancellation unless an explicit
`cancellation` is passed; an already-cancelled token fails the call before any
blocking work.

## Credentials

`pi.auth.v1` stores and resolves provider credentials. Both file locations are
passed in, so no path, provider name, or precedence rule exists in Rust:

- `store{canonical=[, legacy=]}` returns a credential store. Both paths must be
  absolute and must differ. `canonical` is the only file ever written; `legacy`
  is a read-only fallback used only while `canonical` is absent;
- `providers()` — the subscription identities that can refresh a stored OAuth
  row (`id`, `name`, `uses_callback_server`), in registry order;
- `api_version`, `max_secret_bytes` (64 KiB), and `max_providers` (256).

A store provides `snapshot()`, `describe(provider)`, `set_api_key(provider,
value)`, `set_oauth(provider, credentials)`, `remove(provider)`, and
`resolve(provider)`.

`snapshot()` reports `source` (`canonical`, `legacy`, or `absent`) and the
stored provider names. `describe(provider)` reports `kind` (`api_key` or
`oauth`) plus, for an OAuth row, `expires`, `expired`, and the provider-defined
`extra_fields` names. Neither ever returns a secret, so credential state can be
rendered without holding one.

`resolve(provider)` is the only member that yields a secret, as `{api_key,
refreshed}` or `nil`. A stored api-key row is an expression: `$NAME` expands
from the process environment and a leading `!` runs the rest through the shell
with its own hard timeout and a process-wide result cache. This expansion is the
provider subsystem's stored-value mechanism and deliberately reads the live
process environment, unlike `pi.effects.v1.env`, which is the immutable startup
snapshot. An expired OAuth row is refreshed through its subscription provider
and written back under the same lock; `refreshed` reports whether that happened.

`set_oauth` requires `refresh`, `access`, and `expires` (epoch milliseconds) and
preserves every other field verbatim as provider-defined extra data. The first
write promotes storage to `canonical` and migrates the selected legacy rows
forward; the legacy file is never modified. Writes are lock-serialized, replace
the canonical file atomically, and keep it owner-private.

Mutating members are asynchronous and observe the innermost dispatch
cancellation. The store holds no operating-system resource between calls, so
there is nothing to dispose: each operation takes the canonical lock, completes
or is cancelled at a lock retry, stored-value command, or token refresh, and
releases it.

## Subscription login

`login(provider, callbacks[, options])` runs one subscription OAuth flow and
returns the credential row it produced — the same shape `set_oauth` accepts.
Nothing is written: which store receives the row, and whether to keep it at all,
stay package decisions.

Rust runs the wire flow (PKCE, the loopback callback server, authorization-code
exchange, RFC 8628 device polling). Every user-visible step is an ordinary Lua
function in `callbacks`:

- `on_auth{url, instructions}` — show the authorization URL (required);
- `on_device_code{user_code, verification_uri, interval_seconds,
  expires_in_seconds}` — show a device code (required);
- `on_prompt{message, placeholder, allow_empty}` returns a line of input
  (required);
- `on_select{message, options={{id, label}, …}}` returns a chosen `id`, or `nil`
  to cancel (required);
- `on_progress(message)` — optional status line;
- `on_manual_code_input()` returns a pasted code or redirect URL. Supplying it is
  what enables the manual path: flows that offer it race manual entry against the
  callback server.

`options` may carry `timeout_ms` (default 900000, maximum 3600000) and
`model_ids(provider)`, which returns the catalog model ids a flow may enable
after a successful login (maximum 128; unsupplied means none). The provider
interface asks for that list synchronously, so it is read once before the flow
starts.

Callbacks are served in arrival order with at most one Lua call in flight, and
concurrently with the flow itself — a pending manual-code prompt does not stop
the callback server from settling. A callback that raises ends the login with
that error. A callback that never returns holds later notifications until the
login settles.

A login observes the innermost dispatch cancellation at every step, including
while parked on a prompt or an HTTP response, and a cancelled login reports as a
cancelled dispatch. It also ends at `timeout_ms`, so no login runs unbounded.
The callback server is owned by the flow future: dropping the login — by
cancellation, timeout, or failure — releases its port.
