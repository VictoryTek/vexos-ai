# THROWAWAY: ACP validation spike

Not part of the product. Results: `.github/docs/subagent_docs/assistant_panel_spike_results.md`.

- `acp_client.py`: a minimal ACP v1 client (Python stdlib). It rejects every
  permission request (`reject_once`, else `reject_always`, else `cancelled`) and
  never selects an allow option.
- `sandbox.sh`: runs a command in a bwrap mount namespace with the managed
  policy files at their real paths (`/etc/claude-code/managed-settings.d/50-vexos.json`,
  `/etc/opencode/opencode.json`) and `nixos-rebuild` shadowed by `stub/`, which
  only logs to `out/nixos-rebuild.calls`. `SPIKE_NO_MANAGED=1` leaves the policy
  files out (control runs).
- `managed/`: the policy files, rendered from this repo's NixOS module:

  ```sh
  nix eval --impure --raw --expr 'let f = builtins.getFlake (toString ./.);
    sys = f.inputs.nixpkgs.lib.nixosSystem { system = "x86_64-linux"; modules = [
      f.nixosModules.default { programs.vexos-ai.enable = true; nixpkgs.config.allowUnfree = true; } ]; };
    in sys.config.environment.etc."<claude-code/managed-settings.d/50-vexos.json|opencode/opencode.json>".text'
  ```
- `out/`: raw JSON-RPC logs (gitignored).
- `fix/routes.sh TAG`: re-runs the settings-file escalation routes (run from
  `spike/acp`; scratch dirs under `$SPIKE_SCRATCH`). `fix/tui.py`: drives the
  OpenCode TUI in tmux. `SPIKE_CLAUDE_MANAGED` / `SPIKE_OPENCODE_MANAGED` point
  `sandbox.sh` at a variant policy file.

Example:

```sh
echo 'run exactly: touch x' | CLAUDE_AGENT_ACP_PROTOCOL=v1 ./sandbox.sh \
  ./acp_client.py --log out/x.jsonl --cwd /abs/scratch -- claude-agent-acp
```
