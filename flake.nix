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
    in
    {
      packages = forAllSystems (pkgs: rec {
        vexos-ai = mkVexosAi pkgs;
        vexos-ai-panel = mkVexosAiPanel pkgs;
        vexos-ai-panel-extension = mkVexosAiExtension pkgs;
        default = vexos-ai;
      });

      overlays.default = final: _prev: { vexos-ai = mkVexosAi final; };

      nixosModules.default = import ./nix/module.nix { inherit mkVexosAi mkVexosAiPanel mkVexosAiExtension; };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ shellcheck shfmt jq bats util-linux cargo rustc clippy rustfmt pkg-config ];
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
        in pkgs.runCommand "vexos-ai-policy" { nativeBuildInputs = [ pkgs.jq ]; } ''
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

        # The GTK window (crates/vexos-ai-panel) builds, and passes clippy. It
        # has no tests: its rules are in the view-model, tested above.
        panel = self.packages.${pkgs.stdenv.hostPlatform.system}.vexos-ai-panel.overrideAttrs (old: {
          nativeBuildInputs = old.nativeBuildInputs ++ [ pkgs.clippy ];
          postCheck = "cargo clippy --offline --all-targets -p vexos-ai-panel -- -D warnings";
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
          diff <(cd ${dir} && find . -type f | sort) <(printf '%s\n' ./extension.js ./layout.js ./metadata.json \
            ./schemas/gschemas.compiled ./schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml)
          node --check ${dir}/extension.js
          gsettings --schemadir ${dir}/schemas list-keys org.gnome.shell.extensions.vexos-assistant \
            | grep -x toggle-assistant >/dev/null
          keys=$(gsettings --schemadir ${dir}/schemas list-keys org.gnome.shell.extensions.vexos-assistant)
          grep -qx window-mode <<<"$keys"
          # The shared value: two choices, docked by default, nothing else storable.
          range=$(gsettings --schemadir ${dir}/schemas range org.gnome.shell.extensions.vexos-assistant window-mode)
          [ "$range" = "$(printf '%s\n' enum "'docked'" "'floating'")" ]
          [ "$(gsettings --schemadir ${dir}/schemas get org.gnome.shell.extensions.vexos-assistant window-mode)" = "'docked'" ]
          export GSETTINGS_BACKEND=memory HOME=$TMPDIR
          if gsettings --schemadir ${dir}/schemas set org.gnome.shell.extensions.vexos-assistant window-mode bogus; then
            echo "window-mode accepted a value outside its choices"; exit 1
          fi
          gsettings --schemadir ${dir}/schemas set org.gnome.shell.extensions.vexos-assistant window-mode floating
          # Size function, validation and the no-agent-text scan, on the built files.
          cp -r ${./tests/extension} tests
          EXT=${dir} node --test tests/layout.test.mjs
          touch $out
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
            "the removed opencodePackage option is refused" = refused "OpenCode was removed" {
              programs.vexos-ai = { enable = true; opencodePackage = stub "opencode" "1.19.0"; };
            };
          };
        in pkgs.runCommand "vexos-ai-module" { } ''
          failed=0
          ${lib.concatStrings (lib.mapAttrsToList (name: ok: ''
            echo ${lib.escapeShellArg ((if ok then "ok   " else "FAIL ") + name)}
            ${lib.optionalString (!ok) "failed=1"}
          '') cases)}
          ${lib.optionalString (panel != null) ''
            grep -q '${agents.claudeAcpPackage}/bin' ${panel}/bin/vexos-ai-panel \
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
          [ "$failed" = 0 ] && touch $out
        '';
      });
    };
}
