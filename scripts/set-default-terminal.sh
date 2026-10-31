#!/bin/sh
# Makes the installed IkTerminal the system default terminal.
#
# GNOME's Ctrl+Alt+T (org.gnome.desktop.default-applications.terminal) already
# runs `x-terminal-emulator -e`, so switching the alternative is enough; no
# extra keybinding is needed on Ubuntu/GNOME.
set -eu
bin=$(command -v ikterminal) || {
    echo "ikterminal not found on PATH (install the .deb first)" >&2
    exit 1
}
if ! update-alternatives --list x-terminal-emulator 2>/dev/null | grep -qx "$bin"; then
    echo "$bin is not registered as an x-terminal-emulator alternative" >&2
    echo "reinstall the .deb, which registers it via packaging/linux/postinst" >&2
    exit 1
fi
sudo update-alternatives --set x-terminal-emulator "$bin"
echo "x-terminal-emulator -> $bin"
echo "Ctrl+Alt+T now opens IkTerminal (GNOME media-keys runs x-terminal-emulator)."
