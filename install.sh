#!/bin/sh
# Instala o pad em ~/.local/bin/pad e liga junto com a sessão gráfica (um
# serviço do systemd do usuário, que volta sozinho se cair).
#
# De dentro do código, compila. Sem o código, baixa o binário da última release:
#   curl -fsSL https://raw.githubusercontent.com/fioravante-dev/pad/main/install.sh | sh
set -eu
REPO=fioravante-dev/pad
BIN="$HOME/.local/bin"
mkdir -p "$BIN" "$HOME/.cache/pad" "$HOME/.config/systemd/user" "$HOME/.local/share/applications"

dir=$(dirname "$0")
if [ -f "$dir/Cargo.toml" ] && command -v cargo >/dev/null; then
    cd "$dir"
    cargo build --release
    install -m755 target/release/pad "$BIN/pad.new"
else
    arch=$(uname -m)
    [ "$arch" = x86_64 ] || { echo "a release só tem x86_64; compile do código (cargo)" >&2; exit 1; }
    echo "baixando a última release de $REPO..."
    curl -fsSL "https://github.com/$REPO/releases/latest/download/pad-linux-x86_64" -o "$BIN/pad.new"
    chmod 755 "$BIN/pad.new"
fi
mv -f "$BIN/pad.new" "$BIN/pad"

# O autostart antigo (.desktop) daria dois pads; o serviço substitui.
rm -f "$HOME/.config/autostart/pad.desktop"

cat >"$HOME/.config/systemd/user/pad.service" <<UNIT
[Unit]
Description=pad: o controle como controle remoto do T3 Code
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart=$BIN/pad
Restart=on-failure
RestartSec=3
StandardOutput=append:$HOME/.cache/pad/pad.log
StandardError=append:$HOME/.cache/pad/pad.log

[Install]
WantedBy=graphical-session.target
UNIT

# No menu de aplicativos: abre a tela de ajustes.
cat >"$HOME/.local/share/applications/pad.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=pad
Comment=Ajustes do controle (o que cada botão faz)
Exec=$BIN/pad ui
Icon=input-gaming
Terminal=false
Categories=Utility;Settings;
DESK

"$BIN/pad" check
systemctl --user daemon-reload
systemctl --user enable pad.service >/dev/null 2>&1
systemctl --user restart pad.service
echo "instalado e rodando (systemctl --user status pad; log em ~/.cache/pad/pad.log)."
