//! O coração: botão apertado → ação; analógicos → mouse e rolagem; combos.
//!
//! Regras dos botões:
//! - fora de combos, a ação dispara ao apertar (o clique segura enquanto o
//!   botão está apertado, para arrastar);
//! - em algum combo, dispara ao soltar, e só se nenhum combo aconteceu
//!   enquanto ele estava apertado. Assim Start+Select+L1+L2 não dita nem
//!   aperta Enter no caminho.

use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use evdev::{Device, FFEffectData, FFEffectKind, FFReplay, FFTrigger};

use crate::config::{Action, Button, Click, Config};
use crate::gamepad::{Axis, Event};
use crate::out::Out;
use crate::status::Shared;
use crate::t3::T3;
use crate::tray::{Handle, Tray};
use crate::watch::Attention;

pub struct Engine {
    cfg: Config,
    out: Out,
    t3: Option<T3>,
    enabled: bool,
    pressed: HashSet<Button>,
    /// Botões que já serviram a um combo (não disparam ao soltar).
    used: HashSet<Button>,
    /// Combos (pelo índice) que estão apertados agora.
    active_combos: HashSet<usize>,
    /// Cliques segurados por botão.
    held_clicks: Vec<(Button, Click)>,
    /// Rolagens seguradas: botão, direção (+1 cima), desde quando.
    held_scrolls: Vec<(Button, i8, Instant)>,
    /// lx, ly, rx, ry.
    axes: [f64; 4],
    device: Option<PathBuf>,
    status: Shared,
    tray: Option<Handle<Tray>>,
}

impl Engine {
    pub fn new(
        cfg: Config,
        out: Out,
        t3: Option<T3>,
        status: Shared,
        tray: Option<Handle<Tray>>,
    ) -> Engine {
        status.lock().unwrap().enabled = cfg.enabled;
        Engine {
            status,
            tray,
            enabled: cfg.enabled,
            cfg,
            out,
            t3,
            pressed: HashSet::new(),
            used: HashSet::new(),
            active_combos: HashSet::new(),
            held_clicks: Vec::new(),
            held_scrolls: Vec::new(),
            axes: [0.0; 4],
            device: None,
        }
    }

    pub fn set_config(&mut self, cfg: Config) {
        if let Some(t) = &self.tray {
            let n = cfg.notify.enabled;
            t.update(|t| t.notify = n);
        }
        let changed = cfg.enabled != self.cfg.enabled;
        let enabled = cfg.enabled;
        self.cfg = cfg;
        if changed {
            self.set_enabled(enabled, false);
        }
        self.active_combos.clear();
    }

    pub fn event(&mut self, ev: Event) {
        match ev {
            Event::Connected { name, path } => {
                eprintln!("[pad] controle: {name} ({})", path.display());
                self.device = Some(path);
                self.status.lock().unwrap().controller = Some(name);
                if self.enabled {
                    self.rumble(1);
                }
            }
            Event::Disconnected => {
                self.device = None;
                self.status.lock().unwrap().controller = None;
                self.release_all();
            }
            Event::Axis(a, v) => {
                let i = match a {
                    Axis::LeftX => 0,
                    Axis::LeftY => 1,
                    Axis::RightX => 2,
                    Axis::RightY => 3,
                };
                self.axes[i] = v;
            }
            Event::Button(b, down) => {
                self.status.lock().unwrap().press(b, down);
                if down { self.press(b) } else { self.release(b) }
            }
            Event::SetEnabled(on) => self.set_enabled(on, true),
            Event::Attention(a) => self.attention(a),
            Event::Quit => {}
        }
    }

    fn press(&mut self, b: Button) {
        if !self.pressed.insert(b) {
            return;
        }
        // Combos: o primeiro (maior) que fechou agora.
        let hit = self.cfg.combos.iter().enumerate().find(|(i, c)| {
            !self.active_combos.contains(i)
                && c.buttons.contains(&b)
                && c.buttons.iter().all(|x| self.pressed.contains(x))
        });
        if let Some((i, combo)) = hit {
            let (buttons, action) = (combo.buttons.clone(), combo.action.clone());
            self.active_combos.insert(i);
            self.used.extend(buttons.iter().copied());
            // Um clique segurado por um botão do combo é solto.
            for x in &buttons {
                self.release_click(*x);
            }
            eprintln!(
                "[pad] combo: {:?}",
                buttons.iter().map(|x| x.name()).collect::<Vec<_>>()
            );
            self.run(&action, None);
            return;
        }
        if self.cfg.in_combo(b) {
            return;
        }
        let action = self.cfg.action(b).clone();
        self.run(&action, Some(b));
    }

    fn release(&mut self, b: Button) {
        if !self.pressed.remove(&b) {
            return;
        }
        let combos = &self.cfg.combos;
        self.active_combos
            .retain(|&i| !combos.get(i).is_some_and(|c| c.buttons.contains(&b)));
        self.release_click(b);
        self.held_scrolls.retain(|(x, _, _)| *x != b);
        let was_used = self.used.remove(&b);
        if self.cfg.in_combo(b) && !was_used {
            let action = self.cfg.action(b).clone();
            match action {
                // Ao soltar, o clique é um toque.
                Action::Click(c) if self.enabled => {
                    let _ = self.out.click(c, true);
                    let _ = self.out.click(c, false);
                }
                other => self.run(&other, None),
            }
        }
    }

    fn release_click(&mut self, b: Button) {
        if let Some(pos) = self.held_clicks.iter().position(|(x, _)| *x == b) {
            let (_, c) = self.held_clicks.remove(pos);
            let _ = self.out.click(c, false);
        }
    }

    fn release_all(&mut self) {
        self.release_all_but_buttons();
        self.pressed.clear();
        self.used.clear();
        self.active_combos.clear();
        self.axes = [0.0; 4];
    }

    /// `holder`: o botão que segura a ação (para o clique soltar junto).
    fn run(&mut self, action: &Action, holder: Option<Button>) {
        if !self.enabled && *action != Action::Toggle {
            return;
        }
        // Com a tela de ajustes aberta, os botões só aparecem nela (o
        // clique segue valendo, para usar a própria tela).
        if !matches!(action, Action::Click(_)) && self.status.lock().unwrap().learning() {
            return;
        }
        if *action != Action::None {
            eprintln!(
                "[pad] {} → {action:?}",
                holder.map_or("combo/soltar", |b| b.name())
            );
        }
        let result = match action {
            Action::None => Ok(()),
            Action::Toggle => {
                self.set_enabled(!self.enabled, true);
                Ok(())
            }
            Action::Talk => {
                self.bring_t3();
                spawn(&self.cfg.talk_command);
                Ok(())
            }
            Action::Focus => {
                self.bring_t3();
                Ok(())
            }
            Action::Keys(keys, t3) => {
                if *t3 {
                    self.bring_t3();
                }
                self.out.keys(keys)
            }
            Action::Click(c) => match holder {
                Some(b) => {
                    self.held_clicks.push((b, *c));
                    self.out.click(*c, true)
                }
                None => self
                    .out
                    .click(*c, true)
                    .and_then(|_| self.out.click(*c, false)),
            },
            Action::Scroll(dir) => {
                if let Some(b) = holder {
                    self.held_scrolls.push((b, *dir, Instant::now()));
                }
                self.out.scroll(f64::from(*dir) * SCROLL_STEP)
            }
            Action::Run(cmd) => {
                spawn(cmd);
                Ok(())
            }
        };
        if let Err(e) = result {
            eprintln!("[pad] falhou: {e:#}");
        }
    }

    /// Liga/desliga; `feedback`: avisa na tela e vibra.
    fn set_enabled(&mut self, on: bool, feedback: bool) {
        if on == self.enabled {
            return;
        }
        self.enabled = on;
        self.release_all_but_buttons();
        self.status.lock().unwrap().enabled = on;
        if let Some(t) = &self.tray {
            t.update(|t| t.enabled = on);
        }
        let msg = if on { "pad ligado" } else { "pad desligado" };
        eprintln!("[pad] {msg}");
        if feedback {
            notify(msg);
            self.rumble(if on { 1 } else { 2 });
        }
    }

    /// Ao desligar, solta cliques e zera o movimento (os botões seguem
    /// contados, para o combo de religar funcionar).
    fn release_all_but_buttons(&mut self) {
        for (_, c) in std::mem::take(&mut self.held_clicks) {
            let _ = self.out.click(c, false);
        }
        self.held_scrolls.clear();
        self.out.reset_motion();
    }

    fn bring_t3(&mut self) {
        let Some(t3) = &self.t3 else { return };
        if t3.is_focused() {
            return;
        }
        match t3.focus() {
            // O COSMIC leva um instante para dar o foco ao T3.
            Ok(true) => thread::sleep(Duration::from_millis(150)),
            Ok(false) => eprintln!("[pad] o T3 não está aberto"),
            Err(e) => eprintln!("[pad] não trouxe o T3: {e:#}"),
        }
    }

    /// Algum analógico fora da zona morta (o laço anda mais rápido).
    pub fn moving(&self) -> bool {
        let dz = self.cfg.mouse.deadzone;
        self.enabled
            && (!self.held_scrolls.is_empty()
                || stick(self.axes[0], self.axes[1], dz, 1.0).is_some()
                || stick(self.axes[2], self.axes[3], dz, 1.0).is_some())
    }

    /// Um passo do mouse / da rolagem; `dt` em segundos.
    pub fn tick(&mut self, dt: f64) {
        if !self.enabled {
            return;
        }
        // Rolagem segurada: depois de um instante, rola contínuo.
        let held: f64 = self
            .held_scrolls
            .iter()
            .filter(|(_, _, since)| since.elapsed() >= SCROLL_HOLD)
            .map(|(_, dir, _)| f64::from(*dir))
            .sum();
        if held != 0.0
            && let Err(e) = self.out.scroll(held * self.cfg.scroll.speed * dt)
        {
            eprintln!("[pad] rolagem falhou: {e:#}");
        }
        let m = &self.cfg.mouse;
        let (lx, ly, rx, ry) = (self.axes[0], self.axes[1], self.axes[2], self.axes[3]);
        // Rolagem: os dois analógicos juntos, para cima ou para baixo.
        if self.cfg.scroll.enabled {
            let vertical = |x: f64, y: f64| y.abs() > 0.5 && x.abs() < 0.5;
            if vertical(lx, ly) && vertical(rx, ry) && ly.signum() == ry.signum() {
                let mag = ((ly.abs() + ry.abs()) / 2.0 - 0.5) / 0.5;
                let lines = -ly.signum() * self.cfg.scroll.speed * mag.clamp(0.1, 1.0) * dt;
                if let Err(e) = self.out.scroll(lines) {
                    eprintln!("[pad] rolagem falhou: {e:#}");
                }
                return;
            }
        }
        let mut dx = 0.0;
        let mut dy = 0.0;
        for (on, x, y) in [
            (self.cfg.left_stick_mouse, lx, ly),
            (self.cfg.right_stick_mouse, rx, ry),
        ] {
            if let (true, Some((vx, vy))) = (on, stick(x, y, m.deadzone, m.curve)) {
                dx += vx;
                dy += vy;
            }
        }
        if (dx != 0.0 || dy != 0.0)
            && let Err(e) = self.out.mouse_move(dx * m.speed * dt, dy * m.speed * dt)
        {
            eprintln!("[pad] mouse falhou: {e:#}");
        }
    }

    /// O T3 precisa de você: vibra como o [notify] pede.
    fn attention(&self, a: Attention) {
        let n = &self.cfg.notify;
        let wanted = match a {
            Attention::Test => true,
            Attention::Finished => n.enabled && n.finished,
            Attention::Question => n.enabled && n.question,
        };
        if !wanted {
            return;
        }
        eprintln!("[pad] T3: {a:?}");
        let mag = |base: f64| (base * n.strength).round().clamp(0.0, 65535.0) as u16;
        // A pergunta pede mais: uma tremida a mais que o fim de uma resposta.
        let extra = u32::from(a == Attention::Question);
        self.shake(n.pulses + extra, n.length_ms as u16, mag(65535.0), mag(52000.0));
    }

    /// Tremidinha no controle: 1 = ligado, 2 = desligado.
    fn rumble(&self, times: u32) {
        self.shake(times, 120, 0x6000, 0xa000);
    }

    fn shake(&self, times: u32, length: u16, strong: u16, weak: u16) {
        let Some(path) = self.device.clone() else {
            return;
        };
        thread::spawn(move || {
            let go = || -> std::io::Result<()> {
                let mut dev = Device::open(&path)?;
                let mut fx = dev.upload_ff_effect(FFEffectData {
                    direction: 0,
                    trigger: FFTrigger::default(),
                    replay: FFReplay { length, delay: 0 },
                    kind: FFEffectKind::Rumble {
                        strong_magnitude: strong,
                        weak_magnitude: weak,
                    },
                })?;
                for _ in 0..times {
                    fx.play(1)?;
                    thread::sleep(Duration::from_millis(u64::from(length) + 130));
                }
                Ok(())
            };
            if let Err(e) = go() {
                eprintln!("[pad] sem vibrar: {e}");
            }
        });
    }
}

/// Linhas de um toque em "scroll".
const SCROLL_STEP: f64 = 3.0;
/// Segurando "scroll" por isto, rola contínuo.
const SCROLL_HOLD: Duration = Duration::from_millis(300);

/// Analógico com zona morta radial e curva: (x, y) de -1 a 1, ou `None` parado.
fn stick(x: f64, y: f64, deadzone: f64, curve: f64) -> Option<(f64, f64)> {
    let mag = x.hypot(y);
    if mag <= deadzone {
        return None;
    }
    let scaled = ((mag - deadzone) / (1.0 - deadzone))
        .clamp(0.0, 1.0)
        .powf(curve);
    Some((x / mag * scaled, y / mag * scaled))
}

/// Roda um comando sem esperar (e sem deixar zumbi).
fn spawn(cmd: &str) {
    match Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .spawn()
    {
        Ok(mut child) => {
            thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("[pad] não rodou {cmd:?}: {e}"),
    }
}

/// Aviso na tela (notificação do COSMIC).
pub fn notify(msg: &str) {
    spawn(&format!(
        "notify-send -a pad -t 2500 'pad' {}",
        shell_quote(msg)
    ));
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadzone_and_curve() {
        assert!(stick(0.1, 0.0, 0.15, 2.0).is_none());
        let (x, y) = stick(1.0, 0.0, 0.15, 2.0).unwrap();
        assert!((x - 1.0).abs() < 1e-9 && y == 0.0);
        let (x, _) = stick(0.5, 0.0, 0.15, 2.0).unwrap();
        assert!(x > 0.0 && x < 0.5);
    }

    #[test]
    fn quote() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
