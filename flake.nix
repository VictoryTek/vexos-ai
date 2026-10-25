{
  description = "vexos-ai — AI assistant (Claude Code / OpenCode) for VexOS";

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
    in
    {
      packages = forAllSystems (pkgs: rec {
        vexos-ai = mkVexosAi pkgs;
        default = vexos-ai;
      });

      overlays.default = final: _prev: { vexos-ai = mkVexosAi final; };

      nixosModules.default = import ./nix/module.nix { inherit mkVexosAi; };

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [ shellcheck shfmt jq bats util-linux cargo rustc clippy rustfmt ];
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
        in pkgs.runCommand "vexos-ai-policy" { nativeBuildInputs = [ pkgs.jq ]; } ''
          check() { jq -e "$2" "$1" >/dev/null || { echo "policy check failed: $2 ($1)"; exit 1; }; }

          check ${claude} '.permissions.disableAutoMode == "disable"'
          check ${claude} 'has("disableAutoMode") | not'
          check ${claude} '.permissions.disableBypassPermissionsMode == "disable"'
          check ${claude} '.permissions.defaultMode == "default"'
          check ${opencode} '.default_agent == "build"'
          for c in "nixos-rebuild switch" "nixos-rebuild boot" "nixos-rebuild test" \
                   "just rebuild" "just switch" "just update" "just update-all" "nix flake check"; do
            for p in "" "sudo " "pkexec "; do
              check ${claude} ".permissions.deny | index(\"Bash($p$c *)\") != null"
              for scope in .permission .agent.build.permission .agent.general.permission \
                           .agent.plan.permission .agent.explore.permission; do
                check ${opencode} "$scope.bash[\"$p$c*\"] == \"deny\""
              done
            done
          done

          # OpenCode uses the last matching rule: "*" must come first and only ask.
          for scope in .permission .agent.build.permission .agent.general.permission \
                       .agent.plan.permission .agent.explore.permission; do
            check ${opencode} "$scope.edit == \"ask\""
            check ${opencode} "($scope.bash | keys_unsorted[0]) == \"*\" and $scope.bash[\"*\"] == \"ask\""
            check ${opencode} "[$scope.bash | to_entries[] | select(.key != \"*\") | .value] | all(. == \"deny\")"
          done
          touch $out
        '';

        # The CLI's machine contract (docs/contract.md), black-box: the real
        # script under a fake HOME, with stub claude/curl/systemctl/zenity/…
        # first on PATH. The script is built without curl, zenity, libnotify and
        # xdg-terminal-exec in its runtime inputs, which would otherwise be put
        # in front of the stubs; everything else is identical to the package.
        contract = let
          vexosAi = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
          script = vexosAi.passthru.mkScript (with pkgs; [
            coreutils findutils gnugrep jq glib util-linux
          ]);
        in pkgs.runCommand "vexos-ai-contract" {
          nativeBuildInputs = with pkgs; [ bats jq coreutils findutils gnugrep util-linux ];
        } ''
          export VEXOS_AI_BIN=${script}/bin/vexos-ai
          cp -r ${./tests/contract} contract
          chmod -R u+w contract
          chmod +x contract/stubs/*
          patchShebangs contract # the sandbox may have no /usr/bin/env
          bats --print-output-on-failure contract
          touch $out
        '';

        # The ACP core (crates/vexos-ai-core): its safety tests against a fake
        # agent, plus clippy. A library, so there is nothing to install.
        core = pkgs.rustPlatform.buildRustPackage {
          pname = "vexos-ai-core";
          version = "0.1.0";
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./crates ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.clippy ];
          postCheck = "cargo clippy --offline --all-targets -- -D warnings";
          installPhase = "touch $out";
        };
      });
    };
}
