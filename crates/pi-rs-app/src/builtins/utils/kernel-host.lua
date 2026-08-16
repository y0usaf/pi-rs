-- kernel-host.lua — Stage 1 of docs/pi-kernel-surface.md: compose the product's
-- two authoritative states onto the ONE VM-resident kernel Context from Lua,
-- instead of Rust `DaemonBoundary::mount_host` / `mount_session`.
--
-- The product daemon hands the live Host and the active-session manager handle
-- to this VM via a KernelBridge (daemon.rs). That bridge is the host-side
-- holder; the kernel Context stores only serde_json-liveness markers, because
-- the real Host / SessionManagerHandle are not serializable and a blocking
-- `Host::stop` would deadlock on the VM thread. This fragment therefore exposes
-- the *composition* — the actual `pi.kernel.mount` that declares
-- `daemon:host:vm` + `session:active` — as a pure function, and the product
-- run invokes it once the daemon is up.
--
-- It is deliberately a function, not a load-time side effect: plain hosts and
-- parity suites load this pack but never call it, so they mount nothing and the
-- default product stays byte-identical / residue-free.
local pi = ...

-- Mount the single reversible kernel Component that composes the product's two
-- authoritative states. Effects commit `daemon:host:vm` (host liveness) and
-- `session:active` (active-session handle); each snapshots the prior (absent)
-- value, so `pi.kernel.unmount` replays the inverse and both keys are removed
-- (residue-free). The real host teardown (`Host::stop`) is marshalled host-side
-- (daemon `retain_host`/`drain`), the only thread where that blocking stop is
-- legal.
local function mount_host_lifecycle()
  local effects = {
    { key = "daemon:host:vm", value = true },
    { key = "session:active", value = true },
  }
  return pi.kernel.mount({ effects = effects })
end

-- Public exact-version module: the Stage 1 kernel fold (PLAN 9.7/9.10 module).
pi.module.define({
  name = "pi.agent.kernel-host",
  version = "1",
  dependencies = {},
  factory = function()
    return {
      mount_host_lifecycle = mount_host_lifecycle,
      daemon_key = "daemon:host:vm",
      session_key = "session:active",
    }
  end,
})