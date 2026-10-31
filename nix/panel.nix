# vexos-ai-panel — the assistant's GTK4/libadwaita window (crates/vexos-ai-panel).
# It runs claude-agent-acp from PATH, like the CLI finds its
# agent. Not installed by nix/module.nix yet.
{ lib, rustPlatform, pkg-config, wrapGAppsHook4, makeDesktopItem, copyDesktopItems
, glib, gtk4, libadwaita, dconf }:

let
  # The extension's schema, installed for the panel too: the panel writes its
  # `window-mode` key, and the Shell extension reads it.
  schema = ../extension + "/vexos-assistant@victorytek.github.io/schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml";
in

rustPlatform.buildRustPackage {
  pname = "vexos-ai-panel";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [ ../Cargo.toml ../Cargo.lock ../crates schema ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [ "-p" "vexos-ai-panel" ];
  cargoTestFlags = [ "-p" "vexos-ai-panel" ];

  nativeBuildInputs = [ pkg-config wrapGAppsHook4 copyDesktopItems glib ];
  # dconf: the wrapper's GIO modules, so GSettings writes reach the Shell.
  buildInputs = [ gtk4 libadwaita dconf ];

  desktopItems = [
    (makeDesktopItem {
      name = "io.github.victorytek.VexosAssistant";
      desktopName = "VexOS Assistant";
      comment = "Ask an AI assistant to change or troubleshoot this system";
      exec = "vexos-ai-panel";
      icon = "utilities-terminal";
      # Reached from the quick settings toggle or a keybind, not the app grid.
      noDisplay = true;
      categories = [ "System" "Utility" ];
      startupWMClass = "io.github.victorytek.VexosAssistant";
    })
  ];

  # D-Bus activation: the Shell extension calls the `toggle` action by app id,
  # which starts the panel if it is not running.
  postInstall = ''
    mkdir -p $out/share/glib-2.0/schemas
    cp ${schema} $out/share/glib-2.0/schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml
    glib-compile-schemas --strict $out/share/glib-2.0/schemas
    mkdir -p $out/share/dbus-1/services
    cat > $out/share/dbus-1/services/io.github.victorytek.VexosAssistant.service <<EOF
    [D-BUS Service]
    Name=io.github.victorytek.VexosAssistant
    Exec=$out/bin/vexos-ai-panel --gapplication-service
    EOF
  '';

  meta = {
    description = "VexOS assistant panel";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "vexos-ai-panel";
  };
}
