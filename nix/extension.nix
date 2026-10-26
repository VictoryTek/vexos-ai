# The GNOME Shell extension for the assistant panel (extension/<uuid>/): a
# Quick Settings tile and a shortcut that show or hide the panel's window.
# Installed by nix/module.nix; each user enables it once with
# `gnome-extensions enable <uuid>`.
{ lib, stdenvNoCC, glib }:

let
  uuid = "vexos-assistant@victorytek.github.io";
  metadata = builtins.fromJSON (builtins.readFile (../extension + "/${uuid}/metadata.json"));
in
stdenvNoCC.mkDerivation {
  pname = "gnome-shell-extension-vexos-assistant";
  version = "1";
  # The uuid has an `@`, which a store path name can't, so the root is the parent.
  src = lib.fileset.toSource { root = ../extension; fileset = ../extension; };
  nativeBuildInputs = [ glib ];

  dontBuild = true;
  installPhase = ''
    runHook preInstall
    dir=$out/share/gnome-shell/extensions/${uuid}
    mkdir -p $dir
    cp -r ${uuid}/. $dir/
    glib-compile-schemas --strict $dir/schemas
    runHook postInstall
  '';

  passthru = { extensionUuid = uuid; inherit metadata; };

  meta = {
    description = "GNOME Shell extension for the VexOS assistant panel";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
  };
}
