#!/usr/bin/env python3
"""Linux smoke client for zapret2-svc (CI step 5 probe).

Speaks the svc_ipc JSON-lines protocol over the Unix socket:
  phase "start": hello -> start(yt4k) -> poll get-status until running
  phase "stop":  stop -> poll get-status until stopped -> shutdown

Prints SMOKE markers; exits non-zero on the first failed expectation so
the workflow log shows exactly which stage broke.
"""
import json
import socket
import sys
import time

SOCK = sys.argv[1]
PHASE = sys.argv[2]
STRATEGY = "yt4k"


def rpc(msg, timeout=10.0):
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.settimeout(timeout)
    s.connect(SOCK)
    s.sendall((json.dumps(msg) + "\n").encode())
    buf = b""
    while b"\n" not in buf:
        chunk = s.recv(65536)
        if not chunk:
            break
        buf += chunk
    s.close()
    return json.loads(buf.split(b"\n")[0].decode())


def status_state():
    r = rpc({"cmd": "get-status"})
    return r.get("status", {}).get("state"), r


def wait_state(want, deadline_s):
    t0 = time.time()
    last = None
    while time.time() - t0 < deadline_s:
        state, r = status_state()
        last = (state, r)
        if state == want:
            return state, r
        time.sleep(2)
    return last


def main():
    r = rpc({"cmd": "hello", "version": 2})
    if r.get("type") != "welcome":
        print(f"SMOKE: hello FAIL: {r}")
        return 1
    print("SMOKE: hello OK")

    if PHASE == "start":
        r = rpc({"cmd": "start", "strategy_id": STRATEGY, "custom_args": None},
                timeout=120.0)
        print(f"SMOKE: start reply: {json.dumps(r)[:400]}")
        if not r.get("ok", False):
            print("SMOKE: start FAIL (service refused)")
            return 1
        state, r = wait_state("running", 90)
        print(f"SMOKE: state after start: {state}")
        if state != "running":
            print(f"SMOKE: running FAIL: {json.dumps(r)[:400]}")
            return 1
        print("SMOKE: running OK")
        return 0

    if PHASE == "stop":
        r = rpc({"cmd": "stop"}, timeout=60.0)
        print(f"SMOKE: stop reply: {json.dumps(r)[:300]}")
        state, r = wait_state("stopped", 60)
        print(f"SMOKE: state after stop: {state}")
        if state != "stopped":
            print("SMOKE: stopped FAIL")
            return 1
        print("SMOKE: stopped OK")
        try:
            rpc({"cmd": "shutdown"}, timeout=5.0)
        except Exception as e:
            print(f"SMOKE: shutdown note: {e}")
        print("SMOKE: shutdown sent")
        return 0

    print(f"SMOKE: unknown phase {PHASE}")
    return 2


if __name__ == "__main__":
    sys.exit(main())
