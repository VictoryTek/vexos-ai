# shellcheck shell=bash
# Sourced by the other harness scripts. The nested shell's home, config, data,
# cache, state and runtime directories are under target/shell-spike-run/ in this
# repository, and it writes nowhere else. It is not fully isolated from the host
# for reading: start.sh passes the host XDG_DATA_DIRS and NIX_GSETTINGS_OVERRIDES_DIR
# (Nix profile and Flatpak share directories: schemas, icons, desktop files), so the
# nested shell briefly loads the host's system extensions before start.sh disables
# them inside it.
REPO=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
RUN=$REPO/target/shell-spike-run
UUID=vexos-shell-spike@victorytek.github.io
EXT_SRC=$REPO/spike/shell-panel/$UUID
export REPO RUN UUID EXT_SRC
