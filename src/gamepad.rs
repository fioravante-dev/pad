//! Acha o controle e lê os botões e analógicos dele (evdev), numa thread.
//!
//! O 8BitDo no modo X aparece como um controle de Xbox (driver xpad). Sem
//! controle, a thread procura de novo a cada 2 s: dá para ligar e desligar o
//! controle com o pad rodando.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use evdev::{AbsoluteAxisCode as Abs, Device, EventSummary, KeyCode};

use crate::config::Button;

/// O nome dos aparelhos que o próprio pad cria (para não ler a si mesmo).
pub const OWN_PREFIX: &str = "pad virtual";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    LeftX,
    LeftY,
    RightX,
    RightY,
}

#[derive(Debug)]
pub enum Event {
    Connected {
        name: String,
        path: PathBuf,
    },
    Disconnected,
    Button(Button, bool),
    /// -1 a 1; y positivo é para baixo.
    Axis(Axis, f64),
}

/// Para normalizar um eixo: o centro e meia volta.
#[derive(Clone, Copy)]
struct Range {
    min: f64,
    max: f64,
}

impl Range {
    fn stick(self, v: i32) -> f64 {
        let mid = (self.min + self.max) / 2.0;
        let half = (self.max - self.min) / 2.0;
        if half <= 0.0 {
            0.0
        } else {
            ((v as f64 - mid) / half).clamp(-1.0, 1.0)
        }
    }
    fn trigger(self, v: i32) -> f64 {
        let span = self.max - self.min;
        if span <= 0.0 {
            0.0
        } else {
            ((v as f64 - self.min) / span).clamp(0.0, 1.0)
        }
    }
}

/// Gatilho analógico: a partir daqui conta como apertado.
const TRIGGER_ON: f64 = 0.5;
const TRIGGER_OFF: f64 = 0.35;

fn is_gamepad(d: &Device) -> bool {
    let name = d.name().unwrap_or("");
    if name.starts_with(OWN_PREFIX) {
        return false;
    }
    let keys = d
        .supported_keys()
        .is_some_and(|k| k.contains(KeyCode::BTN_SOUTH));
    let axes = d
        .supported_absolute_axes()
        .is_some_and(|a| a.contains(Abs::ABS_X) && a.contains(Abs::ABS_RX));
    keys && axes
}

fn find(filter: &str) -> Option<(PathBuf, Device)> {
    let filter = filter.to_lowercase();
    let mut all: Vec<(PathBuf, Device)> =
        evdev::enumerate().filter(|(_, d)| is_gamepad(d)).collect();
    all.sort_by(|a, b| a.0.cmp(&b.0));
    all.into_iter()
        .find(|(_, d)| filter.is_empty() || d.name().unwrap_or("").to_lowercase().contains(&filter))
}

fn button(code: KeyCode) -> Option<Button> {
    Some(match code {
        KeyCode::BTN_SOUTH => Button::A,
        KeyCode::BTN_EAST => Button::B,
        // O xpad troca norte e oeste: BTN_NORTH é o X do Xbox.
        KeyCode::BTN_NORTH => Button::X,
        KeyCode::BTN_WEST => Button::Y,
        KeyCode::BTN_TL => Button::L1,
        KeyCode::BTN_TR => Button::R1,
        KeyCode::BTN_TL2 => Button::L2,
        KeyCode::BTN_TR2 => Button::R2,
        KeyCode::BTN_THUMBL => Button::L3,
        KeyCode::BTN_THUMBR => Button::R3,
        KeyCode::BTN_SELECT => Button::Select,
        KeyCode::BTN_START => Button::Start,
        KeyCode::BTN_MODE => Button::Home,
        KeyCode::BTN_DPAD_UP => Button::Up,
        KeyCode::BTN_DPAD_DOWN => Button::Down,
        KeyCode::BTN_DPAD_LEFT => Button::Left,
        KeyCode::BTN_DPAD_RIGHT => Button::Right,
        _ => return None,
    })
}

/// Sobe a thread. `filter` é o pedaço do nome do controle ([device]).
pub fn spawn(filter: String, tx: Sender<Event>) {
    thread::Builder::new()
        .name("gamepad".into())
        .spawn(move || {
            let mut warned = false;
            loop {
                let Some((path, dev)) = find(&filter) else {
                    if !warned {
                        eprintln!("[pad] nenhum controle; procuro de 2 em 2 s");
                        warned = true;
                    }
                    thread::sleep(Duration::from_secs(2));
                    continue;
                };
                warned = false;
                let name = dev.name().unwrap_or("controle").to_string();
                if tx.send(Event::Connected { name, path }).is_err() {
                    return;
                }
                if let Err(e) = read(dev, &tx) {
                    eprintln!("[pad] o controle saiu: {e}");
                }
                if tx.send(Event::Disconnected).is_err() {
                    return;
                }
                thread::sleep(Duration::from_millis(500));
            }
        })
        .expect("thread do controle");
}

fn read(mut dev: Device, tx: &Sender<Event>) -> std::io::Result<()> {
    let state = dev.get_abs_state()?;
    let range = |a: Abs| {
        let s = state[a.0 as usize];
        Range {
            min: s.minimum as f64,
            max: s.maximum as f64,
        }
    };
    let (lx, ly, rx, ry) = (
        range(Abs::ABS_X),
        range(Abs::ABS_Y),
        range(Abs::ABS_RX),
        range(Abs::ABS_RY),
    );
    let (lz, rz) = (range(Abs::ABS_Z), range(Abs::ABS_RZ));
    let mut trig = [false, false];
    let mut hat = [0i32, 0i32];
    let send = |e: Event| {
        tx.send(e)
            .map_err(|_| std::io::Error::other("o pad fechou"))
    };
    loop {
        for ev in dev.fetch_events()? {
            match ev.destructure() {
                EventSummary::Key(_, code, v) if v != 2 => {
                    if let Some(b) = button(code) {
                        send(Event::Button(b, v == 1))?;
                    }
                }
                EventSummary::AbsoluteAxis(_, a, v) => match a {
                    Abs::ABS_X => send(Event::Axis(Axis::LeftX, lx.stick(v)))?,
                    Abs::ABS_Y => send(Event::Axis(Axis::LeftY, ly.stick(v)))?,
                    Abs::ABS_RX => send(Event::Axis(Axis::RightX, rx.stick(v)))?,
                    Abs::ABS_RY => send(Event::Axis(Axis::RightY, ry.stick(v)))?,
                    Abs::ABS_Z | Abs::ABS_RZ => {
                        let (i, r, b) = if a == Abs::ABS_Z {
                            (0, lz, Button::L2)
                        } else {
                            (1, rz, Button::R2)
                        };
                        let t = r.trigger(v);
                        let now = if trig[i] {
                            t > TRIGGER_OFF
                        } else {
                            t >= TRIGGER_ON
                        };
                        if now != trig[i] {
                            trig[i] = now;
                            send(Event::Button(b, now))?;
                        }
                    }
                    Abs::ABS_HAT0X | Abs::ABS_HAT0Y => {
                        let i = usize::from(a == Abs::ABS_HAT0Y);
                        let (neg, pos) = if i == 0 {
                            (Button::Left, Button::Right)
                        } else {
                            (Button::Up, Button::Down)
                        };
                        let old = hat[i];
                        let new = v.signum();
                        if old != new {
                            if old < 0 {
                                send(Event::Button(neg, false))?;
                            } else if old > 0 {
                                send(Event::Button(pos, false))?;
                            }
                            if new < 0 {
                                send(Event::Button(neg, true))?;
                            } else if new > 0 {
                                send(Event::Button(pos, true))?;
                            }
                            hat[i] = new;
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
