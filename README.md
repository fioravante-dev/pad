# pad

O controle (8BitDo no modo X, ou qualquer controle de Xbox) como controle
remoto do T3 Code, no Linux com COSMIC. Leve: uma thread lendo o controle,
~2 MB de RAM, CPU parada quando os analógicos estão parados.

- **L1 / R1**: traz o T3 para a frente (mesmo minimizado) e dispara o ditado
  do talk na caixinha onde você estava.
- **L2 / R2**: Enter. **Direcional ↑ ↓**: conversa anterior/próxima;
  **← →**: rolar para cima/baixo (segure para continuar).
- **Os dois analógicos**: mouse.
- **Start + Select + L1 + L2**: liga/desliga o pad (vibra 1x ligado, 2x desligado).

Tudo muda em `~/.config/pad/config.toml` (criado na primeira vez, com um
comentário em cada chave). Salvou, valeu: o pad relê sozinho.

## Instalar

```sh
./install.sh          # compila, põe em ~/.local/bin/pad e no autostart
pad check             # confere o config.toml
tail -f ~/.cache/pad/pad.log
```

Precisa da sessão gráfica (WAYLAND_DISPLAY) e de acesso a `/dev/uinput` e ao
controle; o logind dá os dois para quem está logado na frente do PC.
