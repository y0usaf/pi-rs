-- Translation of Pi v0.79.0 examples/extensions/dynamic-resources/index.ts.
-- Loads skills, prompts, and themes using the resources_discover event.
--
-- Pi resolves the asset directory from the extension's own source (import.meta
-- / fileURLToPath). pi-rs exposes no import.meta to extensions; this translation
-- announces the companion assets shipped next to the translation under
-- examples/extensions/dynamic-resources/ (relative to the project cwd), which
-- preserves the resources_discover contract: three path arrays returned to the
-- resource loader.
local pi = ...

local base_dir = pi.path.join(pi.cwd(), "examples", "extensions", "dynamic-resources")

pi.on("resources_discover", function()
  return {
    skillPaths = { pi.path.join(base_dir, "SKILL.md") },
    promptPaths = { pi.path.join(base_dir, "dynamic.md") },
    themePaths = { pi.path.join(base_dir, "dynamic.json") },
  }
end)