#!/usr/bin/env python3
"""run.py [--port N] [--token T] FILE.jsonl — run control requests from a JSON-lines file
({"method":..., "params":...} or {"sleep": s}) against a running LightCraft's control server.

--port defaults to $LIGHTCRAFT_CONTROL_PORT, else 7980; --token (added to every request as
"token") defaults to $LIGHTCRAFT_CONTROL_TOKEN, else none."""
import argparse, json, os, socket, time


def parse_args():
    ap = argparse.ArgumentParser(description="Run LightCraft control requests from a JSON-lines file.")
    ap.add_argument("script", help="JSON-lines file: one request per line, or {\"sleep\": seconds}")
    ap.add_argument("--port", type=int, default=int(os.environ.get("LIGHTCRAFT_CONTROL_PORT") or 7980),
                    help="control port (default: $LIGHTCRAFT_CONTROL_PORT, else 7980)")
    ap.add_argument("--token", default=os.environ.get("LIGHTCRAFT_CONTROL_TOKEN") or None,
                    help="token sent with every request (default: $LIGHTCRAFT_CONTROL_TOKEN)")
    return ap.parse_args()


def call(port, token, method, params):
    request = {"id": 1, "method": method, "params": params}
    if token:
        request["token"] = token
    s = socket.create_connection(("127.0.0.1", port), timeout=180)
    s.sendall((json.dumps(request) + "\n").encode())
    buf = b""
    while not buf.endswith(b"\n"):
        c = s.recv(1 << 20)
        if not c: break
        buf += c
    return json.loads(buf)


def main():
    args = parse_args()
    for line in open(args.script):
        line = line.strip()
        if not line or line.startswith("#"): continue
        r = json.loads(line)
        if "sleep" in r:
            time.sleep(r["sleep"]); continue
        out = call(args.port, args.token, r["method"], r.get("params", {}))
        print(r["method"], json.dumps(out)[:160])


if __name__ == "__main__":
    main()
