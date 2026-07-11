-- Exerciser for the OS bindings: pi.fs.*, pi.path.*, pi.env, pi.cwd().
--
-- In pi these are ambient Node (node:fs, node:path, process.env); pi-rs
-- exposes them as bindings so translations stay mechanical
-- (fs.readFileSync(p) → pi.fs.read_file(p)). The fs calls are async on
-- the coroutine seam — the awaits below are invisible and watchdog-free.
local pi = ...

pi.register_command("os-demo", {
  description = "Walk the fs/path/env bindings in a scratch dir",
  handler = function()
    local dir = pi.path.join(pi.env.TMPDIR or "/tmp", "pi-rs-os-demo")
    pi.fs.mkdir(dir)

    local file = pi.path.join(dir, "notes.txt")
    pi.fs.write_file(file, "line 1\n")
    pi.fs.append_file(file, "line 2\n")

    local text = pi.fs.read_file(file)
    local raw = pi.fs.read_bytes(file) -- Buffer read: binary-safe Lua string
    local real = pi.fs.realpath(dir) -- canonical path (symlinks resolved)
    local normalized = pi.text.nfkc("ＡＢＣ１２３") -- JS String.normalize("NFKC") mechanism
    local decoded = pi.json.decode([[{"ok":true}]]) -- JSON.parse mechanism
    local st = pi.fs.stat(file)
    local names = pi.fs.read_dir(dir)
    pi.fs.remove_file(file)

    return ("cwd=%s base=%s ext=%s bytes=%d raw=%d real_base=%s lines=%d entries=%d exists_after=%s"):format(
      pi.cwd(),
      pi.path.basename(file),
      pi.path.extname(file),
      st.size,
      #raw,
      pi.path.basename(real),
      select(2, text:gsub("\n", "\n")),
      #names,
      tostring(pi.fs.exists(file)) .. ":" .. normalized .. ":" .. tostring(decoded.ok)
    )
  end,
})
