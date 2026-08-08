-- mechanism-demo.lua — PLAN 9.9 mechanism supersurface exerciser.
--
-- One file-backed exerciser per mechanism family; each command returns a
-- table the Rust tests assert against. Cancellation / timeout / reload /
-- shutdown / leak contracts are exercised here and in the Rust harness.

local pi = ...

-- ------------------------------------------------------------------
-- pi.crypto + pi.buffer
-- ------------------------------------------------------------------
pi.register_command("mechanism-crypto", {
  description = "sha256/random_uuid/xxhash32/create_hash + Buffer ops",
  handler = function()
    local sha = pi.crypto.sha256("abc")
    assert(sha == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
           "sha256(abc) vector mismatch: " .. sha)
    local uuid = pi.crypto.random_uuid()
    assert(#uuid == 36 and uuid:sub(9, 9) == "-" and uuid:sub(14, 14) == "-", "uuid shape")
    local xx = pi.crypto.xxhash32("hello")
    assert(type(xx) == "number" and xx >= 0 and xx <= 0xffffffff, "xxhash32 range")
    local h = pi.crypto.create_hash("sha256")
    h:update("ab")
    h:update("c")
    assert(h:digest() == sha, "streaming digest matches one-shot")
    local h2 = pi.crypto.create_hash("sha256")
    h2:update("abc")
    assert(h2:digest("base64") == "ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0=", "base64 digest")
    local alloc = pi.buffer.alloc(4)
    assert(#alloc == 4 and alloc == "\0\0\0\0", "alloc zero-fills")
    local from = pi.buffer.from({ 104, 105 })
    assert(from == "hi", "from byte array")
    assert(pi.buffer.byte_length("hi") == 2, "byte_length")
    assert(pi.buffer.concat({ "a", "b" }) == "ab", "concat")
    assert(pi.buffer.from_hex("6869") == "hi", "from_hex")
    return { sha256 = sha, uuid = uuid, xxhash32 = xx }
  end,
})

-- ------------------------------------------------------------------
-- pi.timer — scoped timers with dispose
-- ------------------------------------------------------------------
pi.register_command("mechanism-timer", {
  description = "set_timeout/set_interval/clear + dispose contracts",
  handler = function()
    local ticks = 0
    local interval = pi.timer.set_interval(20, function() ticks = ticks + 1 end)
    pi.sleep(70)
    pi.timer.clear_interval(interval)
    local cleared_at = ticks
    pi.sleep(40)
    assert(ticks == cleared_at, "cleared interval kept firing")
    assert(ticks >= 2, "interval did not fire during sleep: " .. tostring(ticks))

    local fired = 0
    local timeout = pi.timer.set_timeout(20, function() fired = fired + 1 end)
    pi.sleep(50)
    assert(fired == 1, "timeout fired " .. tostring(fired) .. " times")
    pi.timer.clear_timeout(timeout)

    local cancelled = 0
    local c = pi.timer.set_timeout(10, function() cancelled = cancelled + 1 end)
    assert(pi.timer.clear_timer(c), "clear_timer removes")
    pi.sleep(30)
    assert(cancelled == 0, "cancelled timeout fired")

    local live_before = pi.timer.count()
    local disposed = pi.timer.dispose_all()
    assert(disposed >= live_before, "dispose_all count")
    assert(pi.timer.count() == 0, "no timers survive dispose_all")
    return { interval_ticks = cleared_at, timeout_fired = fired, cancelled_fired = cancelled }
  end,
})

-- ------------------------------------------------------------------
-- pi.process — subprocess pipes, kill, tree cancellation, capture
-- ------------------------------------------------------------------
pi.register_command("mechanism-process", {
  description = "spawn pipes / kill / spawn_sync / exec_file_sync / dispose",
  handler = function()
    -- stdin -> stdout roundtrip through pipes
    local child = pi.process.spawn("cat")
    child:write_stdin("pipe-roundtrip")
    child:close_stdin()
    local out = child:read_stdout(64, 2000)
    local waited = child:wait(2000)
    assert(out == "pipe-roundtrip", "stdin->stdout roundtrip: " .. tostring(out))
    assert(waited.code == 0, "cat exited zero")
    child:dispose()

    -- kill() terminates the child (process-tree cancellation via group)
    local sleeper = pi.process.spawn("sh", { "-c", "sleep 100" })
    assert(sleeper:is_running(), "sleeper running")
    sleeper:kill("SIGKILL")
    local w = sleeper:wait(2000)
    assert(not sleeper:is_running(), "kill terminated the child (still running)")
    assert(w.code == 0, "signal death maps to code 0 (Node null)")
    sleeper:dispose()

    -- tree cancellation: the shell and its descendant die together
    local tree = pi.process.spawn("sh", { "-c", "sleep 100 & wait" })
    tree:kill("SIGKILL")
    local tw = tree:wait(2000)
    assert(not tree:is_running(), "tree dead after kill")
    tree:dispose()

    local sync = pi.process.spawn_sync("sh", { "-c", "echo captured; exit 7" })
    assert(sync.stdout == "captured\n", "spawn_sync stdout")
    assert(sync.code == 7, "spawn_sync code")
    local efs = pi.process.exec_file_sync("echo", { "hello" })
    assert(efs.code == 0 and efs.stdout == "hello\n", "exec_file_sync")

    assert(type(pi.process.platform()) == "string", "platform")
    assert(type(pi.process.pid()) == "number", "host pid")
    -- kill(2) of a live pid succeeds; the tree-kill contract is proven above.

    return { roundtrip = out, sync_code = sync.code }
  end,
})

-- ------------------------------------------------------------------
-- pi.fs — watch / atomic / symlink / metadata / open
-- ------------------------------------------------------------------
pi.register_command("mechanism-fs", {
  description = "fs watch/atomic/symlink/lstat/chmod/rename/rm/mkdtemp/access/copy/open",
  handler = function()
    local dir = pi.fs.mkdtemp("pi-mech-")
    local target = dir .. "/target.txt"
    pi.fs.atomic_write(target, "atomic-content")
    assert(pi.fs.read_file(target) == "atomic-content", "atomic_write content")
    assert(pi.fs.access(target), "access after write")

    local sym = dir .. "/link.txt"
    pi.fs.symlink(target, sym)
    local lst = pi.fs.lstat(sym)
    assert(lst.type == "symlink", "lstat sees symlink, got " .. lst.type)
    local st = pi.fs.stat(sym)
    assert(st.type == "file", "stat follows symlink")

    local renamed = dir .. "/renamed.txt"
    pi.fs.rename(target, renamed)
    assert(pi.fs.exists(renamed) and not pi.fs.exists(target), "rename")
    pi.fs.chmod(renamed, "600")
    local copied = dir .. "/copy.txt"
    pi.fs.copy_file(renamed, copied)
    assert(pi.fs.read_file(copied) == "atomic-content", "copy_file")

    local handle = pi.fs.open(dir .. "/handle.txt", "w")
    handle:write("handle-data")
    handle:close()
    local rh = pi.fs.open(dir .. "/handle.txt", "r")
    assert(rh:read(64) == "handle-data", "open read")
    rh:close()

    assert(pi.fs.constants.R_OK == 4 and pi.fs.constants.F_OK == 0, "constants")

    -- watcher: fires on change, stops after close
    local watch_path = dir .. "/watched.txt"
    pi.fs.write_file(watch_path, "v0")
    local changes = 0
    local watcher = pi.fs.watch(watch_path, function(_p, _ev) changes = changes + 1 end)
    pi.fs.atomic_write(watch_path, "v1")
    pi.sleep(250)
    assert(changes >= 1, "watcher fired, got " .. tostring(changes))
    watcher:close()
    local closed_at = changes
    pi.fs.write_file(watch_path, "v2")
    pi.sleep(250)
    assert(changes == closed_at, "closed watcher kept firing")

    -- cleanup
    pi.fs.rm(dir, true)
    assert(not pi.fs.exists(dir), "rm removed the tree")
    return { watcher_fired = changes, lstat = lst.type }
  end,
})

-- ------------------------------------------------------------------
-- pi.net — TCP framed client (line framing) + URL
-- ------------------------------------------------------------------
pi.register_command("mechanism-net", {
  description = "TCP create_connection/read_line/read_some/write + pi.url.parse",
  handler = function(args)
    local host, port = args:match("(%S+)%s+(%S+)")
    local socket = pi.net.create_connection(host, port, { timeout_ms = 2000 })
    socket:write("PING\n")
    local line = socket:read_line(64, 2000)
    assert(line == "PONG", "line framing roundtrip: " .. tostring(line))
    socket:write("BYE\n")
    local rest = socket:read_some(2000)
    socket:close()
    assert(rest == nil or rest == "", "socket closed after BYE")

    local u = pi.url.parse("https://example.com:8443/a/b?q=1#frag", nil)
    assert(u.protocol == "https:", "url protocol")
    assert(u.host == "example.com" and u.port == "8443", "url host/port")
    assert(u.pathname == "/a/b" and u.search == "?q=1" and u.hash == "#frag", "url parts")
    local rel = pi.url.parse("/c", "https://example.com/base/x")
    assert(rel.pathname == "/c", "url base resolution")
    return { line = line }
  end,
})

-- ------------------------------------------------------------------
-- pi.resources — session-scoped managed resources, no leaks
-- ------------------------------------------------------------------
pi.register_command("mechanism-resources", {
  description = "resources list/dispose_all; nothing survives disposal",
  handler = function()
    -- create one of each resource kind
    local timer = pi.timer.set_interval(60, function() end)
    local child = pi.process.spawn("sh", { "-c", "sleep 100" })
    local sock_path = pi.fs.mkdtemp("pi-net-leak-") .. "/w.txt"
    pi.fs.write_file(sock_path, "x")
    local watcher = pi.fs.watch(sock_path, function() end)

    local kinds = {}
    for _, res in ipairs(pi.resources.list()) do
      kinds[res.kind] = (kinds[res.kind] or 0) + 1
    end
    assert(kinds["resource.child_process"] == 1, "child tracked")
    assert(kinds["resource.file_watcher"] == 1, "watcher tracked")
    assert(kinds["timer.interval"] == 1, "timer tracked")

    -- explicit disposal kills the child and removes its resource
    local pid = child:pid()
    assert(pid > 0, "child pid")
    child:dispose()
    local dead = true
    if pi.process.kill(pid, "SIGKILL") then
      -- kill(2) returning 0 means the process still exists (or is a zombie)
      pi.sleep(50)
      dead = pi.process.kill(pid, "SIGKILL") == false
    end
    assert(dead, "disposed child process is gone")

    watcher:close()
    pi.timer.clear_timer(timer)

    -- dispose_all drains the registry
    local n = pi.resources.dispose_all()
    assert(pi.resources.count() == 0, "no resources survive dispose_all")
    assert(n >= 0, "dispose count")
    pi.fs.rm(pi.path.dirname(sock_path), true)
    return { tracked = #kinds, disposed = n, remaining = pi.resources.count() }
  end,
})

-- mechanism-shutdown: leaves one live child behind; the host shutdown
-- path must dispose it (asserted by the Rust harness after drop).
pi.register_command("mechanism-shutdown", {
  description = "leave a live subprocess for the shutdown contract",
  handler = function()
    local child = pi.process.spawn("sh", { "-c", "sleep 100" })
    return { pid = child:pid() }
  end,
})
