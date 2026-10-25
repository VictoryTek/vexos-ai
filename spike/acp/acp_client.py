#!/usr/bin/env python3
"""THROWAWAY SPIKE: minimal ACP v1 client (stdlib only).

Every session/request_permission is answered with a rejection (reject_once,
else reject_always, else cancelled). It never selects an allow option. It
advertises no fs or terminal capability and refuses those requests.

Usage: acp_client.py --log out/x.jsonl [--mode ID] [--meta JSON]
                     [--watch DIR] [--cwd DIR] -- AGENT-CMD... <<< prompts
Prompts come one per line on stdin; each is sent as its own turn.
"""
import argparse, json, os, subprocess, sys, threading, time, queue

ap = argparse.ArgumentParser()
ap.add_argument("--log", required=True)
ap.add_argument("--cwd", default=os.getcwd())
ap.add_argument("--mode", action="append", default=[], help="session/set_mode to try, in order")
ap.add_argument("--config", action="append", default=[], help="ID=VALUE for session/set_config_option")
ap.add_argument("--meta", help="JSON for session/new _meta")
ap.add_argument("--watch", help="dir whose listing is logged at each permission request")
ap.add_argument("--timeout", type=float, default=300)
ap.add_argument("cmd", nargs="+")
a = ap.parse_args()
prompts = [l.rstrip("\n") for l in sys.stdin if l.strip()]

log = open(a.log, "w")
def L(kind, obj):
    log.write(json.dumps({"t": round(time.time(), 3), "kind": kind, "msg": obj}) + "\n"); log.flush()

p = subprocess.Popen(a.cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                     stderr=open(a.log + ".stderr", "w"), cwd=a.cwd, text=True, bufsize=1)
pending, nid, lock = {}, [0], threading.Lock()
summary = {"permission_requests": [], "tool_calls": {}, "text": ""}

def send(obj):
    L("out", obj); p.stdin.write(json.dumps(obj) + "\n"); p.stdin.flush()

def request(method, params):
    with lock:
        nid[0] += 1; i = nid[0]; q = queue.Queue(); pending[i] = q
    send({"jsonrpc": "2.0", "id": i, "method": method, "params": params})
    try:
        return q.get(timeout=a.timeout)
    except queue.Empty:
        return {"error": {"message": "spike client timeout"}}

def on_permission(msg):
    params = msg["params"]
    opts = params.get("options", [])
    pick = next((o for k in ("reject_once", "reject_always") for o in opts if o.get("kind") == k), None)
    listing = sorted(os.listdir(a.watch)) if a.watch else None
    tc = params.get("toolCall", {})
    summary["permission_requests"].append({"title": tc.get("title"), "rawInput": tc.get("rawInput"),
        "options": [(o.get("kind"), o.get("name")) for o in opts], "answer": pick and pick["kind"],
        "watch_dir_at_request": listing})
    time.sleep(2)  # give anything that started early a chance to show up
    outcome = {"outcome": "selected", "optionId": pick["optionId"]} if pick else {"outcome": "cancelled"}
    send({"jsonrpc": "2.0", "id": msg["id"], "result": {"outcome": outcome}})

def reader():
    for line in p.stdout:
        try:
            msg = json.loads(line)
        except ValueError:
            L("in-raw", line); continue
        L("in", msg)
        if "method" in msg and "id" in msg:  # agent -> client request
            if msg["method"] == "session/request_permission":
                threading.Thread(target=on_permission, args=(msg,), daemon=True).start()
            else:
                send({"jsonrpc": "2.0", "id": msg["id"], "error": {"code": -32601, "message": "not supported by spike client"}})
        elif "method" in msg:  # notification
            u = msg.get("params", {}).get("update", {})
            k = u.get("sessionUpdate")
            if k == "agent_message_chunk":
                summary["text"] += u.get("content", {}).get("text", "")
            elif k in ("tool_call", "tool_call_update"):
                t = summary["tool_calls"].setdefault(u["toolCallId"], {})
                for f in ("title", "kind", "status", "rawInput"):
                    if u.get(f) is not None: t[f] = u[f]
                if u.get("content"): t["content"] = json.dumps(u["content"])[:600]
        elif "id" in msg and msg["id"] in pending:
            pending.pop(msg["id"]).put(msg)
threading.Thread(target=reader, daemon=True).start()

res = {}
res["initialize"] = request("initialize", {"protocolVersion": 1,
    "clientCapabilities": {"fs": {"readTextFile": False, "writeTextFile": False}, "terminal": False},
    "clientInfo": {"name": "vexos-ai-spike", "version": "0"}})
new = {"cwd": a.cwd, "mcpServers": []}
if a.meta: new["_meta"] = json.loads(a.meta)
res["session/new"] = request("session/new", new)
sid = res["session/new"].get("result", {}).get("sessionId")
for m in a.mode:
    res[f"set_mode {m}"] = request("session/set_mode", {"sessionId": sid, "modeId": m})
for c in a.config:
    k, v = c.split("=", 1)
    res[f"set_config {c}"] = request("session/set_config_option", {"sessionId": sid, "configId": k, "value": v})
for i, pr in enumerate(prompts):
    if sid:
        res[f"prompt {i}"] = request("session/prompt", {"sessionId": sid, "prompt": [{"type": "text", "text": pr}]})

def short(r):
    s = json.dumps(r.get("result", r.get("error")))
    return s if len(s) < 1500 else s[:1500] + "…"
out = {"responses": {k: short(v) for k, v in res.items()}, **summary}
p.stdin.close()
try: p.wait(timeout=10)
except subprocess.TimeoutExpired: p.kill()
json.dump(out, open(a.log.replace(".jsonl", ".summary.json"), "w"), indent=2)
print(json.dumps(out, indent=2))
