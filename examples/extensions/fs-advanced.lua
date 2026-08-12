-- Exerciser for the extended filesystem mechanisms: symlink/symlink metadata,
-- chmod, rename, atomic write, raw readlink, mkdtemp, and the pollable file
-- watcher (pi.fs.watch_file + handle:poll/close).
--
-- Translations from Node/bun built-ins the dogfood suite relies on:
--   fs.readlink / fs.lstat / fs.chmod / fs.rename / fs.unlink / fs.access
--   fs.mkdtemp / fs.copyFileSync        → pi.fs.*
--   fs.rmSync / fs.rmdir                → pi.fs.remove_dir_all / pi.fs.remove_dir
--   fs.watchFile / fs.unwatchFile       → pi.fs.watch_file + handle:poll/close
--   atomic writes (Hashline)            → pi.fs.write_file_atomic
local pi = ...

pi.register_command("fs-advanced", {
  description = "Exercise symlink/metadata/atomic/rename fs mechanisms",
  handler = function(arg)
    local dir = arg
    local file = pi.path.join(dir, "data.txt")
    local link = pi.path.join(dir, "data-link.txt")
    pi.fs.write_file_atomic(file, "atomic content\n")
    -- symlink + readlink + lstat (symlink identity preserved)
    pi.fs.symlink("data.txt", link)
    local link_target = pi.fs.readlink(link)
    local lst = pi.fs.lstat(link)
    local st = pi.fs.stat(link)
    -- chmod
    pi.fs.chmod(file, "600")
    local mode = pi.fs.lstat(file).mode
    -- rename then read back
    local moved = pi.path.join(dir, "renamed.txt")
    pi.fs.rename(file, moved)
    local content = pi.fs.read_file(moved)
    -- mkdtemp + copy_file + access
    local tmp = pi.fs.mkdtemp(pi.path.join(dir, "tmp-"))
    local tmpfile = pi.path.join(tmp, "copy.txt")
    pi.fs.copy_file(moved, tmpfile)
    local can_access = pcall(pi.fs.access, tmpfile)
    -- unlink + readlink is now broken
    pi.fs.unlink(tmpfile)
    -- rmdir + rmSync: remove the single (now empty) temp dir, then a nested
    -- dir tree via remove_dir_all (Gecko's temp-profile cleanup).
    local single = pi.path.join(dir, "single")
    pi.fs.mkdir(single)
    pi.fs.remove_dir(single)
    local nested = pi.path.join(dir, "nested")
    pi.fs.mkdir(nested)
    pi.fs.write_file(pi.path.join(nested, "a.txt"), "a")
    pi.fs.mkdir(pi.path.join(nested, "sub"))
    pi.fs.write_file(pi.path.join(nested, "sub", "b.txt"), "b")
    pi.fs.remove_dir_all(nested)
    return {
      link_target = link_target,
      lstat_is_symlink = lst.type == "symlink",
      stat_follows_to_file = st.type == "file",
      mode = mode,
      content = content,
      tmp_exists = pi.fs.exists(tmp),
      can_access = can_access,
      moved_exists = pi.fs.exists(moved),
      single_removed = not pi.fs.exists(single),
      nested_removed = not pi.fs.exists(nested),
    }
  end,
})

-- File watcher: watch a path, mutate it on a background task, then poll the
-- handle to fire the callback.
pi.register_command("fs-watch", {
  description = "Exercise pi.fs.watch_file poll/close lifecycle",
  handler = function(arg)
    local path = arg
    pi.fs.write_file(path, "v1\n")
    local events = {}
    local watch = pi.fs.watch_file(path, function(e)
      events[#events + 1] = e.kind
    end)
    -- No event yet.
    local before = watch:poll()
    -- Mutate the file (on the same coroutine; the watcher polls on a thread).
    pi.fs.write_file(path, "v2\n")
    -- Give the background watcher a moment to detect the change.
    pi.sleep(400)
    local fired = watch:poll()
    watch:close()
    local closed = watch:poll() == false
    return { before = before, fired = fired, kinds = events, closed = closed }
  end,
})