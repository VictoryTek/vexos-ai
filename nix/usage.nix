# vexos-ai-usage — the usage service (crates/vexos-ai-usage): on request, it
# reads Claude's plan limits for the top-bar panel and writes numbers to a
# private file. D-Bus activated by the Shell extension, exits when idle, and
# reads nothing at all while plan limits are off (the extension schema's
# `usage-enabled` key; on by default, with an off link in the panel).
{ lib, rustPlatform, pkg-config, wrapGAppsNoGuiHook, glib, dconf }:

let
  # The extension's schema, installed for the service too: it reads the
  # `usage-enabled` key, and the Shell extension writes it.
  schema = ../extension + "/vexos-assistant@victorytek.github.io/schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml";
in

rustPlatform.buildRustPackage {
  pname = "vexos-ai-usage";
  version = "0.1.0";
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [ ../Cargo.toml ../Cargo.lock ../crates schema ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  cargoBuildFlags = [ "-p" "vexos-ai-usage" ];
  cargoTestFlags = [ "-p" "vexos-ai-usage" ];

  nativeBuildInputs = [ pkg-config wrapGAppsNoGuiHook glib ];
  # dconf: the wrapper's GIO modules, so the service reads the user's setting.
  buildInputs = [ glib dconf ];

  # D-Bus activation: the Shell extension calls the `refresh` action by app id,
  # which starts the service if it is not running.
  postInstall = ''
    mkdir -p $out/share/glib-2.0/schemas
    cp ${schema} $out/share/glib-2.0/schemas/org.gnome.shell.extensions.vexos-assistant.gschema.xml
    glib-compile-schemas --strict $out/share/glib-2.0/schemas
    mkdir -p $out/share/dbus-1/services
    cat > $out/share/dbus-1/services/io.github.victorytek.VexosAssistant.Usage.service <<EOF
    [D-BUS Service]
    Name=io.github.victorytek.VexosAssistant.Usage
    Exec=$out/bin/vexos-ai-usage --gapplication-service
    EOF
  '';

  meta = {
    description = "VexOS usage service";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "vexos-ai-usage";
  };
}
