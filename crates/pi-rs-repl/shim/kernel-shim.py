#!/usr/bin/env python3
"""pi-rs kernel shim: IPython InteractiveShell over length-prefixed JSON stdio.

Replaces ipykernel's ZMQ transport with a framed JSON-lines stdio protocol
(pi-rs-repl framing: 4-byte big-endian length + one JSON object) while
keeping IPython cell semantics (display, magics, rich reprs, _ history,
async cells). The vendored `rlm` package keeps its public API; its
host_request is redirected from Jupyter comms to this protocol's
host_request/host_response frames.

Wire version: 1. Protocol documented in crates/pi-rs-repl/src/protocol.rs.

Host -> shim:
  execute      {v, id, code, max_chars}
  interrupt    {v, id}
  host_response{v, req_id, status: "ok"|"error", payload}
  snapshot     {v, id, path, manifest_path, max_bytes}
  restore      {v, id, path}
  shutdown     {v, id}

Shim -> host:
  ready        {v}
  stream       {v, id, name: "stdout"|"stderr", chunk}
  result       {v, id, ok, value, error, duration_ms}
  host_request {v, req_id, kind, payload}
  snapshot_data{v, id, ok, value, error}
  restore_data {v, id, ok, value, error}
"""

from __future__ import annotations

import asyncio
import json
import os
import struct
import sys
import threading
import time
import traceback

VERSION = 1

# The real stdout, captured before any cell replaces sys.stdout with a
# capture wrapper: frame writes must always reach the pipe.
_REAL_STDOUT = sys.stdout

# --- framing -------------------------------------------------------------

def read_frame():
    hdr = sys.stdin.buffer.read(4)
    if not hdr:
        return None
    (n,) = struct.unpack(">I", hdr)
    if n <= 0 or n > 64 * 1024 * 1024:
        raise ValueError(f"bad frame length: {n}")
    return json.loads(sys.stdin.buffer.read(n))


def write_frame(msg):
    data = json.dumps(msg, separators=(",", ":")).encode("utf-8")
    _REAL_STDOUT.buffer.write(struct.pack(">I", len(data)) + data)
    _REAL_STDOUT.buffer.flush()


# --- stdout/stderr capture ------------------------------------------------

class _Capture:
    """Splits writes between the real stream and per-execution buffers."""

    def __init__(self, real, name, on_chunk):
        self._real = real
        self._name = name
        self._on_chunk = on_chunk

    def write(self, text):
        if text:
            self._on_chunk(text, self._name)
        return len(text)

    def flush(self):
        try:
            self._real.flush()
        except Exception:
            pass

    def isatty(self):
        return False

    @property
    def buffer(self):
        # IPython may write raw bytes to sys.stdout.buffer; route to the real
        # stream so frame writes and cell output stay on the pipe.
        return self._real.buffer


# --- display_data interception --------------------------------------------

DIFF_MIME = "application/vnd.prime-agent.diff+json"
ATTACHMENT_MIME = "application/vnd.prime-agent.attachment+json"
AGENT_MESSAGE_MIME = "application/vnd.prime-agent.agent-message+json"
MAX_ATTACHMENT_CHARS = 10_000_000


class _DisplayPub:
    """Captures the three typed display MIME payloads; passes everything else through."""

    def __init__(self, real_pub, on_payload):
        self._real = real_pub
        self._on_payload = on_payload

    def publish(self, data, metadata=None):
        if isinstance(data, dict):
            for mime in (DIFF_MIME, ATTACHMENT_MIME, AGENT_MESSAGE_MIME):
                if mime in data:
                    self._on_payload(mime, data[mime])
                    return
        self._real.publish(data, metadata or {})

    def flush(self):
        self._real.flush()

    # IPython's InteractiveShell.write() consults these on the display_pub to
    # decide whether output goes to display_data or to the stream. We are a
    # plain shell: never publishing, never pprinting.
    is_publishing = False
    is_pprint = False


# --- host bridge -----------------------------------------------------------

class _HostBridge:
    """Speaks host_request/host_response over the stdio framing."""

    def __init__(self):
        self._lock = threading.Lock()
        self._pending = {}
        self._next_req = 0
        self._loop = None

    def attach_loop(self, loop):
        self._loop = loop

    def request(self, kind, payload):
        """Called from rlm's host_request (async context); awaits the reply."""
        if self._loop is None:
            raise RuntimeError("host bridge not attached to an event loop")
        with self._lock:
            self._next_req += 1
            req_id = self._next_req
            future = self._loop.create_future()
            self._pending[req_id] = future
        write_frame({"v": VERSION, "type": "host_request", "req_id": req_id,
                     "kind": kind, "payload": payload or {}})
        return future

    def resolve(self, req_id, status, payload):
        with self._lock:
            future = self._pending.pop(req_id, None)
        if future is None:
            return  # stale reply (interrupted kernel); drop
        if status == "ok":
            future.set_result(payload)
        else:
            future.set_exception(RuntimeError(str(payload.get("error", "host request failed"))))


_bridge = _HostBridge()


# Redirect the vendored rlm package's host_request to the stdio bridge. The
# rlm package is pinned oracle source; we never edit it — this assignment
# replaces its comm-based transport with ours.
def _install_rlm_bridge():
    try:
        import rlm as _rlm
    except Exception:
        return  # rlm optional in bare smoke mode
    async def _stdio_host_request(request_type, payload=None):
        reply = await _bridge.request(request_type, payload or {})
        if not isinstance(reply, dict):
            raise RuntimeError(f"host request {request_type} returned a non-record reply")
        status = reply.get("status")
        if status == "error":
            raise RuntimeError(str(reply.get("error") or f"host request {request_type} failed"))
        if status != "ok":
            raise RuntimeError(f"host request {request_type} returned unexpected status: {status!r}")
        return {k: v for k, v in reply.items() if k != "status"}
    _rlm.host_request = _stdio_host_request


# --- execution -------------------------------------------------------------

class _CellRunner:
    """Runs one cell in the InteractiveShell and collects its outputs."""

    def __init__(self, ip, exec_id, max_chars, stream_cb):
        self._ip = ip
        self._exec_id = exec_id
        self._max_chars = max_chars
        self._stream_cb = stream_cb
        self._stdout = []
        self._stderr = []
        self._stdout_trunc = False
        self._stderr_trunc = False
        self._result = None
        self._diffs = []
        self._attachments = []
        self._sent_messages = []
        self._error = None
        self._status = "ok"

    def _chunk(self, text, name):
        self._stream_cb(text, name, self._exec_id)
        buf = self._stdout if name == "stdout" else self._stderr
        limit = self._max_chars
        if sum(len(b) for b in buf) + len(text) > limit:
            buf.append(text[: max(0, limit - sum(len(b) for b in buf))])
            if name == "stdout":
                self._stdout_trunc = True
            else:
                self._stderr_trunc = True
        else:
            buf.append(text)

    def _on_display(self, mime, payload):
        if mime == DIFF_MIME:
            if isinstance(payload, dict) and isinstance(payload.get("path"), str)                     and isinstance(payload.get("old_str"), str) and isinstance(payload.get("new_str"), str):
                entry = {"path": payload["path"], "old_str": payload["old_str"], "new_str": payload["new_str"]}
                if isinstance(payload.get("start_line"), int):
                    entry["start_line"] = payload["start_line"]
                self._diffs.append(entry)
        elif mime == ATTACHMENT_MIME:
            if isinstance(payload, dict) and isinstance(payload.get("mime_type"), str)                     and isinstance(payload.get("data"), str):
                if len(payload["data"]) > MAX_ATTACHMENT_CHARS:
                    raise RuntimeError("attachment display payload exceeds 10MB cap")
                entry = {"mime_type": payload["mime_type"], "data": payload["data"]}
                if isinstance(payload.get("path"), str):
                    entry["path"] = payload["path"]
                self._attachments.append(entry)
        elif mime == AGENT_MESSAGE_MIME:
            self._sent_messages.append(payload)

    def run(self, code, state):
        started = time.monotonic()
        old_stdout, old_stderr = sys.stdout, sys.stderr
        sys.stdout = _Capture(old_stdout, "stdout", self._chunk)
        sys.stderr = _Capture(old_stderr, "stderr", self._chunk)

        # Interrupt delivery: a per-line trace hook raises KeyboardInterrupt
        # inside exec when the interrupt flag is set. Robust across IPython
        # versions (9.9 schedules sync cells through asyncio machinery where
        # interrupt_main's KI can be swallowed or never land). Blocks in C
        # code (time.sleep) bypass the tracer; interrupt_main remains the
        # backstop.
        def _interrupt_tracer(frame, event, arg):
            if state["interrupt"]:
                raise KeyboardInterrupt()
            return _interrupt_tracer

        sys.settrace(_interrupt_tracer)
        sys.settrace(_interrupt_tracer)
        try:
            result = self._ip.run_cell(code)
        except KeyboardInterrupt:
            self._status = "aborted"
        except Exception as exc:  # run_cell normally captures; belt-and-braces
            self._status = "error"
            self._error = {"ename": type(exc).__name__, "evalue": str(exc),
                           "traceback": traceback.format_exc().splitlines()}
        finally:
            sys.settrace(None)
            sys.stdout, sys.stderr = old_stdout, old_stderr
        # IPython 9 ExecutionResult: success + error_in_exec/error_before_exec
        # (no .status attribute). A KeyboardInterrupt inside exec reports
        # aborted; the interrupt flag below is the backstop for the KI that
        # gets swallowed by the asyncio machinery.
        if result is not None and not result.success:
            err = result.error_in_exec or result.error_before_exec
            if isinstance(err, KeyboardInterrupt):
                self._status = "aborted"
                self._error = None
            elif err is not None:
                self._status = "error"
                self._error = {"ename": type(err).__name__, "evalue": str(err),
                               "traceback": traceback.format_exception(err)}
        # Last expression value: ExecutionResult.result is authoritative
        # (None for assignment-only cells, so no cross-cell leakage).
        out = getattr(result, "result", None) if result is not None else None
        if out is not None and self._status == "ok":
            try:
                self._result = self._ip.display_formatter.format(out).get("text/plain", str(out))
            except Exception:
                self._result = str(out)
        # An interrupt frame may have arrived mid-cell. The KeyboardInterrupt
        # can be swallowed by the asyncio machinery around run_cell, so the
        # flag is the authoritative signal: an interrupted cell is aborted,
        # whatever run_cell reported.
        if state["interrupt"]:
            self._status = "aborted"
            self._error = None
        return {
            "stdout": "".join(self._stdout),
            "stderr": "".join(self._stderr),
            "stdout_truncated": self._stdout_trunc,
            "stderr_truncated": self._stderr_trunc,
            "result": self._result,
            "diffs": self._diffs,
            "attachments": self._attachments,
            "sent_agent_messages": self._sent_messages,
            "status": self._status,
            "error": self._error,
            "duration_ms": int((time.monotonic() - started) * 1000),
        }


# --- snapshot / restore -----------------------------------------------------

RESULT_MARKER = "__PRIME_AGENT_KERNEL_STATE__"


def _snapshot(ip, path, manifest_path, max_bytes):
    """Best-effort per-variable dill snapshot of the user namespace (mirrors
    state-snapshot.ts buildSnapshotCode). Returns the SnapshotResult dict."""
    import builtins as _b
    import json as _json
    import os as _os

    try:
        import dill
    except Exception as _err:
        return {"error": f"dill unavailable: {_err}"}
    dill.settings["recurse"] = True
    ns = ip.user_ns
    hidden = set(getattr(ip, "user_ns_hidden", {}) or {})
    always_skip = {"rlm", "asyncio", "In", "Out", "get_ipython", "exit", "quit", "open"}
    payload = {}
    skipped = []
    total = 0
    for name in list(ns.keys()):
        if name.startswith("_") or name in hidden or name in always_skip:
            continue
        value = ns[name]
        try:
            blob = dill.dumps(value)
        except Exception as _err:
            skipped.append({"name": name, "reason": f"{type(_err).__name__}: {str(_err)[:200]}"})
            continue
        if len(blob) > max_bytes or total + len(blob) > max_bytes:
            skipped.append({"name": name, "reason": "exceeds snapshot size cap"})
            continue
        payload[name] = blob
        total += len(blob)
    _os.makedirs(_os.path.dirname(path), exist_ok=True)
    tmp = path + ".tmp"
    try:
        with _b.open(tmp, "wb") as fh:
            dill.dump(payload, fh)
        _os.replace(tmp, path)
    except Exception as _err:
        try:
            _os.remove(tmp)
        except Exception:
            pass
        return {"error": f"write failed: {_err}"}
    saved = sorted(payload.keys())
    manifest = {"version": 1, "savedNames": saved,
                "skipped": skipped, "bytes": _os.path.getsize(path)}
    try:
        with _b.open(manifest_path, "w") as fh:
            _json.dump(manifest, fh)
    except Exception as _err:
        return {"error": f"manifest write failed: {_err}"}
    return {"saved": saved, "skipped": skipped, "bytes": _os.path.getsize(path), "path": path}


def _restore(ip, path):
    """Inject a dill snapshot payload back into the user namespace."""
    import builtins as _b
    import json as _json

    if not os.path.exists(path):
        return {"error": f"snapshot not found: {path}"}
    try:
        import dill
    except Exception as _err:
        return {"error": f"dill unavailable: {_err}"}
    dill.settings["recurse"] = True
    try:
        with _b.open(path, "rb") as fh:
            payload = dill.load(fh)
    except Exception as _err:
        return {"error": f"read failed: {_err}"}
    restored, failed = [], []
    for name, blob in payload.items():
        try:
            ip.user_ns[name] = dill.loads(blob)
            restored.append(name)
        except Exception as _err:
            failed.append({"name": name, "reason": f"{type(_err).__name__}: {str(_err)[:200]}"})
    return {"restored": restored, "failed": failed, "path": path}


# --- main loop ----------------------------------------------------------------

def main():
    # Bootstrap mirror (RLM_BOOTSTRAP_BASE_CODE in tools/ipython.ts):
    os.environ.setdefault("NO_COLOR", "1")
    try:
        import nest_asyncio
        nest_asyncio.apply()
    except Exception:
        pass
    _install_rlm_bridge()

    import IPython
    ip = IPython.InteractiveShell.instance()
    ip.colors = "nocolor"
    real_pub = ip.display_pub
    ip.display_pub = _DisplayPub(real_pub, lambda mime, payload: _display_cb(mime, payload))

    # IPython stores the last expression value in user_ns["_"]; the runner
    # reads it after each cell.

    _state = {"current": None, "interrupt": False}

    def _display_cb(mime, payload):
        runner = _state["current"]
        if runner is not None:
            runner._on_display(mime, payload)

    loop = asyncio.new_event_loop()
    asyncio.set_event_loop(loop)
    _bridge.attach_loop(loop)

    # Reader thread owns stdin: schedules work onto the main loop.
    def reader():
        while True:
            try:
                frame = read_frame()
            except Exception:
                write_frame({"v": VERSION, "type": "ready", "error": "framing failure"})
                break
            if frame is None:
                break
            ftype = frame.get("type")
            if ftype == "execute":
                loop.call_soon_threadsafe(_do_execute, frame)
            elif ftype == "host_response":
                req_id = frame.get("req_id")
                status = frame.get("status")
                payload = frame.get("payload", {})
                loop.call_soon_threadsafe(_bridge.resolve, req_id, status, payload)
            elif ftype == "interrupt":
                # Flag + deliver KeyboardInterrupt into the running cell's
                # thread. The flag is authoritative (the KI can be swallowed
                # by the asyncio machinery); interrupt_main is the backstop
                # for cells blocked in C code where the flag check never runs.
                _state["interrupt"] = True
                import _thread
                _thread.interrupt_main()
            elif ftype == "snapshot":
                loop.call_soon_threadsafe(_do_snapshot, frame)
            elif ftype == "restore":
                loop.call_soon_threadsafe(_do_restore, frame)
            elif ftype == "shutdown":
                loop.call_soon_threadsafe(loop.stop)
                break

    def _do_execute(frame):
        exec_id = frame.get("id")
        code = frame.get("code", "")
        max_chars = int(frame.get("max_chars", 65536))
        runner = _CellRunner(ip, exec_id, max_chars, _stream_cb)
        _state["current"] = runner
        _state["interrupt"] = False
        try:
            value = runner.run(code, _state)
        except Exception as exc:
            value = {"stdout": "", "stderr": "", "result": None, "diffs": [],
                     "attachments": [], "sent_agent_messages": [],
                     "status": "error",
                     "error": {"ename": type(exc).__name__, "evalue": str(exc),
                               "traceback": traceback.format_exc().splitlines()},
                     "duration_ms": 0}
        finally:
            _state["current"] = None
        write_frame({"v": VERSION, "type": "result", "id": exec_id, "ok": True,
                     "value": value, "error": None, "duration_ms": value["duration_ms"]})

    def _stream_cb(text, name, exec_id):
        write_frame({"v": VERSION, "type": "stream", "id": exec_id, "name": name, "chunk": text})



    def _do_snapshot(frame):
        path = frame.get("path")
        manifest = frame.get("manifest_path")
        max_bytes = int(frame.get("max_bytes", 256 * 1024 * 1024))
        try:
            value = _snapshot(ip, path, manifest, max_bytes)
            write_frame({"v": VERSION, "type": "snapshot_data", "id": frame.get("id"),
                         "ok": "error" not in value, "value": value, "error": value.get("error")})
        except Exception as exc:
            write_frame({"v": VERSION, "type": "snapshot_data", "id": frame.get("id"),
                         "ok": False, "value": None, "error": str(exc)})

    def _do_restore(frame):
        path = frame.get("path")
        try:
            value = _restore(ip, path)
            write_frame({"v": VERSION, "type": "restore_data", "id": frame.get("id"),
                         "ok": "error" not in value, "value": value, "error": value.get("error")})
        except Exception as exc:
            write_frame({"v": VERSION, "type": "restore_data", "id": frame.get("id"),
                         "ok": False, "value": None, "error": str(exc)})

    write_frame({"v": VERSION, "type": "ready"})
    t = threading.Thread(target=reader, daemon=True)
    t.start()
    try:
        loop.run_forever()
    except KeyboardInterrupt:
        pass
    finally:
        try:
            sys.stdout.buffer.flush()
        except Exception:
            pass


if __name__ == "__main__":
    main()
