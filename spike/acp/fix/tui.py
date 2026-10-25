#!/usr/bin/env python3
"""Drive the OpenCode TUI in tmux: report the starting agent, optionally Tab to
an agent, send a prompt, and wait for the stub or a timeout.
Usage: tui.py NAME CWD ENVSPEC TAB_TO PROMPT   (ENVSPEC: "" or "VAR=value")"""
import subprocess, sys, time, os
name, cwd, envspec, tab_to, prompt = sys.argv[1:6]
calls = "/home/nimda/Projects/vexos-ai/spike/acp/out/nixos-rebuild.calls"
sandbox = "/home/nimda/Projects/vexos-ai/spike/acp/sandbox.sh"
def tmux(*a): return subprocess.run(["tmux", *a], capture_output=True, text=True).stdout
def screen(): return tmux("capture-pane", "-p", "-t", name)
def wait(pred, secs):
    end = time.time() + secs
    while time.time() < end:
        if pred(): return True
        time.sleep(1)
    return False
cmd = f"cd {cwd} && env {envspec} {sandbox} opencode"
tmux("new-session", "-d", "-s", name, "-x", "160", "-y", "45", cmd)
wait(lambda: "Build" in screen() or "Yolo" in screen() or "yolo" in screen(), 60)
time.sleep(2)
s = screen()
start = next((w for w in ("Yolo", "yolo", "Build", "Plan") if w in s), "?")
print(f"[{name}] starting agent shown: {start}")
if tab_to:
    for _ in range(4):
        if tab_to.lower() in screen().lower().split("ctrl")[0][-400:].lower() or tab_to.capitalize() in screen(): break
        tmux("send-keys", "-t", name, "Tab"); time.sleep(1.5)
    print(f"[{name}] after Tab: {'yes' if tab_to.capitalize() in screen() or tab_to in screen() else 'not found'} ({tab_to})")
tmux("send-keys", "-t", name, "-l", prompt); time.sleep(0.5); tmux("send-keys", "-t", name, "Enter")
ran = wait(lambda: os.path.exists(calls), 150)
time.sleep(2)
tail = [l for l in screen().splitlines() if l.strip()][-14:]
print(f"[{name}] stub called: {'YES' if ran else 'no'}")
print("\n".join("    | " + l[:150] for l in tail))
tmux("kill-session", "-t", name)
