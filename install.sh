#!/bin/sh
# Compila, instala em ~/.local/bin/pad e liga junto com a sessão (autostart).
set -eu
cd "$(dirname "$0")"
cargo build --release
mkdir -p "$HOME/.local/bin" "$HOME/.config/autostart" "$HOME/.cache/pad"
install -m755 target/release/pad "$HOME/.local/bin/pad.new"
mv -f "$HOME/.local/bin/pad.new" "$HOME/.local/bin/pad"
cat >"$HOME/.config/autostart/pad.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=pad
Comment=O controle como controle remoto do T3 Code
Exec=sh -c "exec $HOME/.local/bin/pad >>$HOME/.cache/pad/pad.log 2>&1"
Terminal=false
NoDisplay=true
X-GNOME-Autostart-enabled=true
EOF
"$HOME/.local/bin/pad" check
echo "instalado. Para (re)iniciar agora: pkill -x pad; setsid -f sh -c 'exec ~/.local/bin/pad >>~/.cache/pad/pad.log 2>&1'"
