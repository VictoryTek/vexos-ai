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
          packages = with pkgs; [ shellcheck shfmt jq ];
        };
      });

      # writeShellApplication runs shellcheck at build time, so building the
      # package is the lint gate.
      checks = forAllSystems (pkgs: {
        vexos-ai = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      });
    };
}
