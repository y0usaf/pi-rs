-- @kernel-demo.lua — the additive `pi.kernel` surface, exercised from a
-- file-backed consumer (docs/pi-kernel-surface.md, Stage 0).
--
-- This mirrors the same spatiotemporal ceremony the kernel and daemon run in
-- Rust (`crates/pi-rs-kernel/src/lib.rs`, `spatiotemporal_ceremony`), but from
-- Lua: mount a reversible kernel Component with reads + an effect, exercise
-- get/has/set/remove and the spatial on_change, then unmount and prove the
-- context returns to its pre-mount state (no residue). `pi.kernel` is a new
-- table member on the existing `pi` table; default pi.* shapes/order/bytes are
-- untouched.
local pi = ...

local report = {}

-- raw get/has/set/remove on the VM-resident write context
pi.kernel.set("probe", "hello")
report.probeGet = pi.kernel.get("probe")
report.probeHas = pi.kernel.has("probe")
report.missingHas = pi.kernel.has("probe.nope")
pi.kernel.remove("probe")
report.removedHas = pi.kernel.has("probe")

-- Pre-mount committed baseline that must survive the mount's inverse replay.
pi.kernel.set("base.one", 1)
pi.kernel.set("base.two", "yes")

-- Mount a reversible component: reads declare the spatial dependency "theme";
-- the effect commits { editor = "idle" }, snapshotting the prior value so the
-- unmount inverse restores it.
local changes = 0
local changedKeys = {}
local id = pi.kernel.mount({
  reads = { "theme" },
  effects = { { key = "editor", value = "idle" } },
  on_change = function(changed_key)
    changes = changes + 1
    changedKeys[changes] = changed_key
  end,
})
report.mountedEditor = pi.kernel.get("editor")
report.editorHas = pi.kernel.has("editor")

-- Spatial on_change: a committed set on a declared read key fires the reaction.
pi.kernel.set("theme", "dark")
report.editorStillIdle = pi.kernel.get("editor") == "idle"
report.onChangeFired = changes == 1
report.changedKey = changedKeys[1]

-- A set on a non-declared key must NOT fire the reaction.
pi.kernel.set("mode", "command")
report.noChangeOnUndeclared = changes == 1

-- Unmount replays the effect inverse in reverse: the editor key returns to its
-- pre-mount (absent) state; pre-mount committed keys are untouched.
pi.kernel.unmount(id)
report.editorGone = pi.kernel.has("editor") == false
report.editorGoneGet = pi.kernel.get("editor")
report.themeStillSet = pi.kernel.get("theme") == "dark"
report.baselineOneStill = pi.kernel.get("base.one") == 1
report.baselineTwoStill = pi.kernel.get("base.two") == "yes"

-- Residue proof: the mount added exactly one key (editor); after unmount it is
-- gone and every pre-mount key/value is exactly as it was.
report.residueFree = report.editorGone
  and report.editorGoneGet == nil
  and report.themeStillSet
  and report.baselineOneStill
  and report.baselineTwoStill

-- A failing assert rejects the chunk, so load_file() fails loudly. The residue
-- diff is therefore proven by the very fact the chunk completes.
assert(pi.kernel.has("editor") == false, "residue: editor survives unmount")
assert(pi.kernel.get("editor") == nil, "residue: editor value survives unmount")
assert(pi.kernel.get("theme") == "dark", "unmount clobbered the pre-mount theme")
assert(pi.kernel.get("base.one") == 1, "unmount drifted base.one")
assert(pi.kernel.get("base.two") == "yes", "unmount drifted base.two")

pi.register_command("kernel-surface-demo", {
  description = "kernel Stage 0 surface report",
  handler = function()
    return report
  end,
})