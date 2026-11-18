{
  description = "vexos-ai — AI assistant (Claude Code) for VexOS";

  inputs = {
    # Same branch as vexos-nix; consumers override with
    # `inputs.vexos-ai.inputs.nixpkgs.follows = "nixpkgs"`.
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
  };

  outputs = { self, nixpkgs }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      mkVexosAi = pkgs: pkgs.callPackage ./nix/package.nix { };
      mkVexosAiPanel = pkgs: pkgs.callPackage ./nix/panel.nix { };
      mkVexosAiExtension = pkgs: pkgs.callPackage ./nix/extension.nix { };
      mkVexosAiUsage = pkgs: pkgs.callPackage ./nix/usage.nix { };
    in
    {
      packages = forAllSystems (pkgs: rec {
        vexos-ai = mkVexosAi pkgs;
        vexos-ai-panel = mkVexosAiPanel pkgs;
        vexos-ai-panel-extension = mkVexosAiExtension pkgs;
        vexos-ai-usage = mkVexosAiUsage pkgs;
        default = vexos-ai;
      });

      overlays.default = final: _prev: { vexos-ai = mkVexosAi final; };

      nixosModules.default = import ./nix/module.nix { inherit mkVexosAi mkVexosAiPanel mkVexosAiExtension mkVexosAiUsage; };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ shellcheck shfmt jq bats util-linux cargo rustc clippy rustfmt pkg-config dbus ];
          # For crates/vexos-ai-panel.
          buildInputs = with pkgs; [ gtk4 libadwaita ];
        };
      });

      # writeShellApplication runs shellcheck at build time, so building the
      # package is the lint gate.
      checks = forAllSystems (pkgs: {
        vexos-ai = self.packages.${pkgs.stdenv.hostPlatform.system}.default;

        # The safety policy as the agents will read it: never apply, always ask.
        policy = let
          policy = import ./nix/policy.nix { inherit (nixpkgs) lib; };
          claude = pkgs.writeText "claude.json" (builtins.toJSON policy.claude);
          opencode = pkgs.writeText "opencode.json" (builtins.toJSON policy.opencode);
          codex = pkgs.writeText "codex.json" (builtins.toJSON policy.codex);
          codexToml = (pkgs.formats.toml { }).generate "requirements.toml" policy.codex;
        in pkgs.runCommand "vexos-ai-policy" { nativeBuildInputs = [ pkgs.jq pkgs.python3 ]; } ''
          check() { jq -e "$2" "$1" >/dev/null || { echo "policy check failed: $2 ($1)"; exit 1; }; }

          check ${claude} '.permissions.disableAutoMode == "disable"'
          check ${claude} 'has("disableAutoMode") | not'
          check ${claude} '.permissions.disableBypassPermissionsMode == "disable"'
          check ${claude} '.permissions.defaultMode == "default"'
          # acceptEdits (also in a subagent's frontmatter) must not skip these.
          for r in Edit Write MultiEdit NotebookEdit \
                   "Bash(mkdir *)" "Bash(touch *)" "Bash(rm *)" "Bash(rmdir *)" \
                   "Bash(mv *)" "Bash(cp *)" "Bash(sed *)"; do
            check ${claude} ".permissions.ask | index(\"$r\") != null"
          done
          for c in "nixos-rebuild switch" "nixos-rebuild boot" "nixos-rebuild test" \
                   "just rebuild" "just switch" "just update" "just update-all" "nix flake check"; do
            for p in "" "sudo " "pkexec "; do
              check ${claude} ".permissions.deny | index(\"Bash($p$c *)\") != null"
            done
          done

          # OpenCode 2.x: edits and shell commands ask, no subagent may start (its
          # own rules are out of reach), the appliers are hard-denied policies, and
          # the 1.x keys are gone (a 2.x OpenCode would not read them).
          check ${opencode} '.permissions == [
            {action: "edit", resource: "*", effect: "ask"},
            {action: "shell", resource: "*", effect: "ask"},
            {action: "subagent", resource: "*", effect: "deny"}]'
          # The pinned agent carries the same rules, so its own come last and say the same.
          check ${opencode} '.agents == {build: {permissions: .permissions}}'
          check ${opencode} '(has("permission") or has("agent") or has("default_agent")) | not'
          # Starting a subagent is a hard policy, not only a rule.
          check ${opencode} '.policies | any(. == {action: "tool.use", resource: "subagent:*", effect: "deny"})'
          check ${opencode} '.policies | all(.action == "tool.use" and .effect == "deny")'
          for c in "nixos-rebuild switch" "nixos-rebuild boot" "nixos-rebuild test" \
                   "just rebuild" "just switch" "just update" "just update-all" "nix flake check"; do
            for p in "" "sudo " "pkexec "; do
              check ${opencode} ".policies | any(.resource == \"shell:$p$c *\")"
            done
          done

          # Codex: always asks the user, read-only sandbox, nothing auto-approves,
          # and every applier (bare, sudo, pkexec) is a forbidden rule.
          # The two policies that ask the user, and no "never".
          check ${codex} '.allowed_approval_policies == ["untrusted", "on-request"]'
          check ${codex} '.allowed_approval_policies | index("never") == null'
          check ${codex} '.allowed_approvals_reviewers == ["user"]'
          check ${codex} '.allowed_sandbox_modes == ["read-only"]'
          check ${codex} '[.rules.prefix_rules[].decision] | all(. == "forbidden")'
          for c in "nixos-rebuild switch" "nixos-rebuild boot" "nixos-rebuild test"                    "just rebuild" "just switch" "just update" "just update-all" "nix flake check"; do
            for p in "" "sudo " "pkexec "; do
              words=$(printf '%s' "$p$c" | jq -cR 'split(" ") | map({token: .})')
              check ${codex} ".rules.prefix_rules | map(.pattern) | any(. == $words)"
            done
          done
          # The TOML Codex reads is the same document as the one asserted above.
          python3 - ${codexToml} ${codex} <<'PY'
          import json, sys, tomllib
          with open(sys.argv[1], "rb") as f:
              toml = tomllib.load(f)
          with open(sys.argv[2]) as f:
              expected = json.load(f)
          assert toml == expected, "the rendered TOML differs from the policy"
          PY
          touch $out
        '';

        # The CLI's machine contract (docs/contract.md), black-box: the real
        # script under a fake HOME, with stub claude/curl/systemctl/zenity/…
        # first on PATH (the curl stub is a tripwire: no command may call it).
        # The script is built without zenity, libnotify and xdg-terminal-exec
        # in its runtime inputs, which would otherwise be put in front of the
        # stubs; everything else is identical to the package.
        contract = let
          vexosAi = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          script = vexosAi.passthru.mkScript (with pkgs; [
            coreutils findutils gnugrep jq glib
          ]);
        in pkgs.runCommand "vexos-ai-contract" {
          nativeBuildInputs = with pkgs; [ bats jq coreutils findutils gnugrep util-linux ];
        } ''
          export VEXOS_AI_BIN=${script}/bin/vexos-ai
          # The source tree, for the scan that no script or module names
          # Claude's login file (tests/contract/credentials.bats).
          export VEXOS_AI_SRC=${self}
          cp -r ${./tests/contract} contract
          chmod -R u+w contract
          chmod +x contract/stubs/*
          patchShebangs contract # the sandbox may have no /usr/bin/env
          bats --print-output-on-failure contract
          touch $out
        '';

        # The ACP core (crates/vexos-ai-core): its safety tests against a fake
        # agent, and the panel's view-model rules (crates/vexos-ai-viewmodel),
        # plus clippy. Neither needs GTK or a display. Libraries, so there is
        # nothing to install.
        core = let
          crates = [ "-p" "vexos-ai-core" "-p" "vexos-ai-viewmodel" ];
        in pkgs.rustPlatform.buildRustPackage {
          pname = "vexos-ai-core";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./crates ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = crates;
          cargoTestFlags = crates;
          nativeBuildInputs = [ pkgs.clippy ];
          postCheck = "cargo clippy --offline --all-targets ${toString crates} -- -D warnings";
          installPhase = "touch $out";
        };

        # The helper (crates/vexos-ai-helper): who may call it and what a call may
        # carry, tested on a private bus where each scenario starts a real
        # dbus-daemon, plus clippy. It needs glib and dbus, which the portable
        # core above does not, so it is a check of its own.
        helper = let
          crate = [ "-p" "vexos-ai-helper" ];
        in pkgs.rustPlatform.buildRustPackage {
          pname = "vexos-ai-helper";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./crates ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = crate;
          # `testing` builds the test-only caller program, never in a release.
          cargoTestFlags = crate ++ [ "--features" "testing" ];
          nativeBuildInputs = [ pkgs.pkg-config pkgs.clippy ];
          buildInputs = [ pkgs.glib ];
          nativeCheckInputs = [ pkgs.dbus ];
          postCheck = "cargo clippy --offline --all-targets ${toString crate} --features testing -- -D warnings";
          installPhase = "touch $out";
        };

        # The GTK window (crates/vexos-ai-panel) builds, and passes clippy. It
        # has no tests: its rules are in the view-model, tested above.
        panel = self.packages.${pkgs.stdenv.hostPlatform.system}.vexos-ai-panel.overrideAttrs (old: {
          nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
          postCheck = "cargo clippy --offline --all-targets -p vexos-ai-panel -- -D warnings";
        });

        # The usage service (crates/vexos-ai-usage): the limits parser, the
        # backoff, the one request's guards (no redirect, size cap, https only),
        # that the token reaches no file or print, and that only one source file
        # names Claude's login file, plus clippy. Its tests run in the build
        # (buildRustPackage's check phase). After install, the installed service
        # is started the way D-Bus starts it: from its own service file, on a
        # private session bus, then asked to refresh with no sign-in around. It
        # must answer and write its record. (A check that only compared the
        # service file's text let a service that exits on start through.) The
        # service file's text is checked in `module`.
        usage = self.packages.${pkgs.stdenv.hostPlatform.system}.vexos-ai-usage.overrideAttrs (old: {
          nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
          postCheck = "cargo clippy --offline --all-targets -p vexos-ai-usage -- -D warnings";
          doInstallCheck = true;
          nativeInstallCheckInputs = [ pkgs.dbus pkgs.glib ];
          installCheckPhase = ''
            export HOME=$TMPDIR/home XDG_STATE_HOME=$TMPDIR/state GSETTINGS_BACKEND=memory
            export XDG_DATA_DIRS=$out/share
            mkdir -p "$HOME"
            unset CLAUDE_CONFIG_DIR
            dbus-run-session --config-file=${pkgs.dbus}/share/dbus-1/session.conf -- bash -ec '
              gdbus call --session --timeout 20 \
                --dest io.github.victorytek.VexosAssistant.Usage \
                --object-path /io/github/victorytek/VexosAssistant/Usage \
                --method org.freedesktop.Application.ActivateAction refresh "[]" "{}"
              for i in $(seq 1 100); do
                [ -s "$XDG_STATE_HOME/vexos-ai/usage.json" ] && break
                sleep 0.2
              done
              cat "$XDG_STATE_HOME/vexos-ai/usage.json"
            ' | tee $TMPDIR/record.json
            # The switch reads as on (the schema default, through the wrapper) and
            # there is no sign-in to read, so the record says so.
            grep -q '"enabled": *true' $TMPDIR/record.json
            grep -q '"status": *"waiting-for-auth"' $TMPDIR/record.json
          '';
        });

        # The extension as built: metadata, layout, JS syntax, compiled schema.
        extension = let
          ext = self.packages.${pkgs.stdenv.hostPlatform.system}.vexos-ai-panel-extension;
          dir = "${ext}/share/gnome-shell/extensions/${ext.extensionUuid}";
        in pkgs.runCommand "vexos-ai-extension" { nativeBuildInputs = with pkgs; [ jq nodejs glib ]; } ''
          set -e
          jq -e '.uuid == "${ext.extensionUuid}" and ."shell-version" == ["50"]
                 and ."settings-schema" == "org.gnome.shell.extensions.vexos-assistant"' ${dir}/metadata.json
          # Exactly these files.
          diff <(cd ${dir} && find . -type f | sort) <(printf '%s\n' ./bus.js ./extension.js ./icons/vexos-assistant-symbolic.svg \
            ./indicator.js ./layout.js ./metadata.json ./schemas/gschemas.compiled \
            ./schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml ./starters.js ./stylesheet.css \
            ./tooltip.js ./usage_store.js ./usage_view.js)
          for f in ${dir}/*.js; do node --check "$f"; done
          gsettings --schemadir ${dir}/schemas list-keys org.gnome.shell.extensions.vexos-assistant \
            | grep -x toggle-assistant >/dev/null
          keys=$(gsettings --schemadir ${dir}/schemas list-keys org.gnome.shell.extensions.vexos-assistant)
          grep -qx window-mode <<<"$keys"
          # The plan-limits switch: on by default (the panel shows the numbers
          # with no click), a plain boolean the panel's off link can clear.
          grep -qx usage-enabled <<<"$keys"
          [ "$(gsettings --schemadir ${dir}/schemas get org.gnome.shell.extensions.vexos-assistant usage-enabled)" = true ]
          [ "$(gsettings --schemadir ${dir}/schemas range org.gnome.shell.extensions.vexos-assistant usage-enabled)" = type\ b ]
          # The shared value: two choices, docked by default, nothing else storable.
          range=$(gsettings --schemadir ${dir}/schemas range org.gnome.shell.extensions.vexos-assistant window-mode)
          [ "$range" = "$(printf '%s\n' enum "'docked'" "'floating'")" ]
          [ "$(gsettings --schemadir ${dir}/schemas get org.gnome.shell.extensions.vexos-assistant window-mode)" = "'docked'" ]
          export GSETTINGS_BACKEND=memory HOME=$TMPDIR
          if gsettings --schemadir ${dir}/schemas set org.gnome.shell.extensions.vexos-assistant window-mode bogus; then
            echo "window-mode accepted a value outside its choices"; exit 1
          fi
          gsettings --schemadir ${dir}/schemas set org.gnome.shell.extensions.vexos-assistant window-mode floating
          # Size function, validation, the usage logic, the source scans (which D-Bus
          # calls, which files, which settings) and the starter list the app shares,
          # on the built files.
          cp -r ${./tests/extension} tests
          cp ${./crates/vexos-ai-viewmodel/src/starters.rs} starters.rs
          EXT=${dir} STARTERS_RS=$PWD/starters.rs node --test tests/*.test.mjs
          touch $out
        '';

        # The skills the module exposes as `programs.vexos-ai.skills`: exactly the
        # two this repo ships, each with a SKILL.md, defined with the module on or
        # off, and nothing linked or published by the module itself.
        skills = let
          inherit (nixpkgs) lib;
          eval = settings: (nixpkgs.lib.nixosSystem {
            inherit (pkgs.stdenv.hostPlatform) system;
            modules = [
              self.nixosModules.default
              { boot.isContainer = true; system.stateVersion = "26.05"; }
              settings
            ];
          }).config;
          off = eval { };
          on = eval { programs.vexos-ai.enable = true; };
          expected = [ "vexos" "vexos-diagnose" ];
          hasSkillMd = skills: lib.all (n: builtins.pathExists (skills.path + "/${n}/SKILL.md")) skills.names;
          cases = {
            "names is exactly vexos and vexos-diagnose (module off)" = off.programs.vexos-ai.skills.names == expected;
            "names is exactly vexos and vexos-diagnose (module on)" = on.programs.vexos-ai.skills.names == expected;
            "each skill has a SKILL.md (module off)" = hasSkillMd off.programs.vexos-ai.skills;
            "each skill has a SKILL.md (module on)" = hasSkillMd on.programs.vexos-ai.skills;
            "path is the skills directory of this source" = off.programs.vexos-ai.skills.path == ./skills;
            "the option is read-only" = !(builtins.tryEval (eval { programs.vexos-ai.skills = { }; }).programs.vexos-ai.skills).success;
            "the module publishes no skills under /etc" = !(on.environment.etc ? "vexos/ai/skills");
            "the module links no skills" =
              !lib.any (r: lib.hasInfix "skills" r) on.systemd.user.tmpfiles.rules;
          };
        in pkgs.runCommand "vexos-ai-skills" { } ''
          failed=0
          ${lib.concatStrings (lib.mapAttrsToList (name: ok: ''
            echo ${lib.escapeShellArg ((if ok then "ok   " else "FAIL ") + name)}
            ${lib.optionalString (!ok) "failed=1"}
          '') cases)}
          [ "$failed" = 0 ] && touch $out
        '';

        # The module's panel contract: the panel comes with the AI feature, never
        # without the managed policy or with untested agents, and it runs the
        # pinned agents. Evaluates the module (stand-in agents, so nothing unfree
        # is fetched) and builds the wrapped panel.
        module = let
          inherit (nixpkgs) lib;
          stub = name: version: pkgs.runCommand "${name}-${version}" { inherit version; } ''
            mkdir -p $out/bin && touch $out/bin/${name}
          '';
          agents = {
            claudePackage = stub "claude" "2.1.300";
            claudeAcpPackage = stub "claude-agent-acp" "0.88.0";
            codexAcpPackage = stub "codex-acp" "2.1.1";
            opencodePackage = stub "opencode" "2.2.6";
          };
          eval = settings: (nixpkgs.lib.nixosSystem {
            inherit (pkgs.stdenv.hostPlatform) system;
            modules = [
              self.nixosModules.default
              { boot.isContainer = true; system.stateVersion = "26.05"; programs.vexos-ai = agents; }
              settings
            ];
          }).config;
          failing = config: lib.concatMapStrings (a: a.message + "\n")
            (lib.filter (a: !a.assertion && lib.hasInfix "programs.vexos-ai" a.message) config.assertions);
          refused = fragment: settings: lib.hasInfix fragment (failing (eval settings));
          panelOf = config: lib.findFirst (p: p.name == "vexos-ai-panel") null config.environment.systemPackages;
          on = eval { programs.vexos-ai.enable = true; };
          off = eval { };
          panel = panelOf on;
          extensionOf = config: lib.findFirst (p: (p.pname or "") == "gnome-shell-extension-vexos-assistant")
            null config.environment.systemPackages;
          gnome = eval { programs.vexos-ai.enable = true; services.desktopManager.gnome.enable = true; };
          extension = extensionOf gnome;
          uuid = "vexos-assistant@victorytek.github.io";
          usageOf = config: lib.findFirst (p: (p.pname or "") == "vexos-ai-usage")
            null config.environment.systemPackages;
          usage = usageOf gnome;
          # Everything the module could enable the extension through.
          enabling = config: lib.concatMapStrings
              (profile: lib.concatMapStrings (db: builtins.toJSON (db.settings or { })) (profile.databases or [ ]))
              (lib.attrValues config.programs.dconf.profiles)
            + config.services.desktopManager.gnome.extraGSettingsOverrides;
          cases = {
            "the panel is registered for D-Bus activation" = lib.elem panel on.services.dbus.packages;
            "the extension comes with the panel on GNOME" =
              gnome.programs.vexos-ai.enablePanelExtension && extension != null && failing gnome == "";
            "no GNOME, no extension" = !on.programs.vexos-ai.enablePanelExtension && extensionOf on == null;
            "no panel, no extension" = extensionOf (eval {
              programs.vexos-ai = { enable = true; enablePanel = false; };
              services.desktopManager.gnome.enable = true;
            }) == null;
            "no extension without the panel" = refused "enablePanelExtension needs" {
              programs.vexos-ai = { enable = true; enablePanel = false; enablePanelExtension = true; };
            };
            "the usage service comes with the extension on GNOME" =
              gnome.programs.vexos-ai.enableUsage && usage != null && lib.elem usage gnome.services.dbus.packages
              && failing gnome == "";
            "no extension, no usage service" = !on.programs.vexos-ai.enableUsage && usageOf on == null;
            "no usage service without the extension" = refused "enableUsage needs" {
              programs.vexos-ai = { enable = true; enableUsage = true; };
            };
            "the module never switches plan limits on" = !lib.hasInfix "usage-enabled" (enabling gnome);
            "the module never enables the extension" = !lib.hasInfix uuid (enabling gnome);
            "the module never sets the window mode" = !lib.hasInfix "window-mode" (enabling gnome);
            "the panel comes with the AI feature" = on.programs.vexos-ai.enablePanel && panel != null && failing on == "";
            "no AI feature, no panel" = !off.programs.vexos-ai.enablePanel && panelOf off == null;
            "the panel can be left out" = panelOf (eval { programs.vexos-ai = { enable = true; enablePanel = false; }; }) == null;
            "no panel without the managed policy" =
              refused "needs programs.vexos-ai.enable" { programs.vexos-ai.enablePanel = true; };
            "no panel without Claude's policy file" = refused "Claude's managed policy" {
              programs.vexos-ai.enable = true;
              environment.etc."claude-code/managed-settings.d/50-vexos.json".enable = lib.mkForce false;
            };
            "no panel with 26.05's claude-agent-acp" = refused "claudeAcpPackage is 0.36.1" {
              programs.vexos-ai = { enable = true; claudeAcpPackage = lib.mkForce (stub "claude-agent-acp" "0.36.1"); };
            };
            "no panel with 26.05's codex-acp" = refused "codexAcpPackage is 0.13.0" {
              programs.vexos-ai = { enable = true; codexAcpPackage = lib.mkForce (stub "codex-acp" "0.13.0"); };
            };
            "no panel with 26.05's opencode" = refused "opencodePackage is 1.15.10" {
              programs.vexos-ai = { enable = true; opencodePackage = lib.mkForce (stub "opencode" "1.15.10"); };
            };
            "no panel with a changed Codex policy" = refused "Codex's managed policy" {
              programs.vexos-ai.enable = true;
              environment.etc."codex/requirements.toml".source = lib.mkForce (pkgs.writeText "x.toml" "");
            };
            "no panel with a changed OpenCode policy" = refused "OpenCode's managed policy" {
              programs.vexos-ai.enable = true;
              environment.etc."opencode/opencode.json".text = lib.mkForce "{}";
            };
            "Codex and OpenCode need nothing set: the policy is installed, nothing else" =
              let bare = (nixpkgs.lib.nixosSystem {
                inherit (pkgs.stdenv.hostPlatform) system;
                modules = [
                  self.nixosModules.default
                  { boot.isContainer = true; system.stateVersion = "26.05";
                    programs.vexos-ai = { enable = true; claudePackage = stub "claude" "2.1.300";
                                          claudeAcpPackage = stub "claude-agent-acp" "0.88.0"; }; }
                ];
              }).config;
              in failing bare == "" && bare.programs.vexos-ai.codexAcpPackage == null
                && bare.programs.vexos-ai.opencodePackage == null
                && bare.environment.etc ? "codex/requirements.toml" && bare.environment.etc ? "opencode/opencode.json";
          };
        in pkgs.runCommand "vexos-ai-module" { } ''
          failed=0
          ${lib.concatStrings (lib.mapAttrsToList (name: ok: ''
            echo ${lib.escapeShellArg ((if ok then "ok   " else "FAIL ") + name)}
            ${lib.optionalString (!ok) "failed=1"}
          '') cases)}
          ${lib.optionalString (panel != null) ''
            grep -q '${agents.claudeAcpPackage}/bin' ${panel}/bin/vexos-ai-panel \
              && grep -q '${agents.codexAcpPackage}/bin' ${panel}/bin/vexos-ai-panel \
              && grep -q '${agents.opencodePackage}/bin' ${panel}/bin/vexos-ai-panel \
              && echo 'ok   the panel runs the pinned agents first on PATH' \
              || { echo 'FAIL the panel runs the pinned agents first on PATH'; failed=1; }
            # The panel reads and writes the extension's `window-mode` key, so it must
            # find the same schema, and reach dconf to write it.
            schemas=$(find ${panel}/share/gsettings-schemas -name 'org.gnome.shell.extensions.vexos-assistant.gschema.xml')
            [ -n "$schemas" ] \
              && cmp "$schemas" ${self}/extension/${uuid}/schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml \
              && test -f "$(dirname "$schemas")/gschemas.compiled" \
              && echo 'ok   the panel installs the extension schema, unchanged and compiled' \
              || { echo 'FAIL the panel installs the extension schema, unchanged and compiled'; failed=1; }
            grep -aq 'dconf-[^ ]*-lib/lib/gio/modules' ${panel}/bin/.vexos-ai-panel-wrapped_ \
              && echo 'ok   the panel can reach dconf' \
              || { echo 'FAIL the panel can reach dconf'; failed=1; }
            test -f ${panel}/share/applications/io.github.victorytek.VexosAssistant.desktop \
              && echo 'ok   the desktop entry is installed' \
              || { echo 'FAIL the desktop entry is installed'; failed=1; }
            grep -qx 'Exec=${panel}/bin/vexos-ai-panel --gapplication-service' \
                ${panel}/share/dbus-1/services/io.github.victorytek.VexosAssistant.service \
              && echo 'ok   D-Bus activation starts the wrapped panel' \
              || { echo 'FAIL D-Bus activation starts the wrapped panel'; failed=1; }
            # GNOME Shell 50 sends a notification click as ActivateAction to the
            # bus name of the app id, once it finds "<app id>.desktop" (NoDisplay
            # is fine; DBusActivatable is not read). So the service's Name= must
            # be the desktop entry's id.
            name=$(sed -n 's/^Name=//p' ${panel}/share/dbus-1/services/io.github.victorytek.VexosAssistant.service)
            [ "$name" = io.github.victorytek.VexosAssistant ] \
              && test -f ${panel}/share/applications/"$name".desktop \
              && ! grep -qx 'Hidden=true' ${panel}/share/applications/"$name".desktop \
              && echo 'ok   notification clicks reach the panel: its bus name is its desktop id' \
              || { echo 'FAIL notification clicks reach the panel: its bus name is its desktop id'; failed=1; }
          ''}
          ${lib.optionalString (extension != null) ''
            test -f ${extension}/share/gnome-shell/extensions/${uuid}/schemas/gschemas.compiled \
              && echo 'ok   the extension is installed with its compiled schema' \
              || { echo 'FAIL the extension is installed with its compiled schema'; failed=1; }
          ''}
          ${lib.optionalString (usage != null) ''
            # D-Bus activation starts the service, under the name the extension calls.
            service=${usage}/share/dbus-1/services/io.github.victorytek.VexosAssistant.Usage.service
            grep -qx 'Name=io.github.victorytek.VexosAssistant.Usage' "$service" \
              && grep -qx 'Exec=${usage}/bin/vexos-ai-usage --gapplication-service' "$service" \
              && echo 'ok   D-Bus activation starts the usage service' \
              || { echo 'FAIL D-Bus activation starts the usage service'; failed=1; }
            # It reads the consent switch from the extension's schema: installed,
            # unchanged and compiled, and reachable through the wrapper.
            schemas=$(find ${usage}/share/gsettings-schemas -name 'org.gnome.shell.extensions.vexos-assistant.gschema.xml')
            [ -n "$schemas" ] \
              && cmp "$schemas" ${self}/extension/${uuid}/schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml \
              && test -f "$(dirname "$schemas")/gschemas.compiled" \
              && echo 'ok   the usage service installs the extension schema, unchanged and compiled' \
              || { echo 'FAIL the usage service installs the extension schema, unchanged and compiled'; failed=1; }
            grep -aq 'dconf-[^ ]*-lib/lib/gio/modules' ${usage}/bin/vexos-ai-usage \
              && echo 'ok   the usage service can reach dconf' \
              || { echo 'FAIL the usage service can reach dconf'; failed=1; }
          ''}
          [ "$failed" = 0 ] && touch $out
        '';
      });
    };
}
