//! Teclado e mouse virtuais (uinput): o que o pad aperta e move.
//!
//! Dois aparelhos, para o libinput tratar cada um como o que é.

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode as Rel};

use crate::config::Click;
use crate::gamepad::OWN_PREFIX;

pub struct Out {
    keyboard: VirtualDevice,
    mouse: VirtualDevice,
    /// Sobras de menos de um pixel / de um passo da roda, para o próximo tique.
    rest_x: f64,
    rest_y: f64,
    rest_wheel: f64,
}

/// A roda de alta resolução: 120 por "linha".
const WHEEL_HI: f64 = 120.0;

impl Out {
    pub fn new() -> Result<Out> {
        let mut keys = AttributeSet::<KeyCode>::new();
        // Todas as teclas comuns de teclado (1..=248).
        for code in 1..=248u16 {
            keys.insert(KeyCode::new(code));
        }
        let keyboard = VirtualDevice::builder()
            .context("sem acesso a /dev/uinput")?
            .name(&format!("{OWN_PREFIX} teclado"))
            .with_keys(&keys)?
            .build()?;

        let mut buttons = AttributeSet::<KeyCode>::new();
        buttons.insert(KeyCode::BTN_LEFT);
        buttons.insert(KeyCode::BTN_RIGHT);
        buttons.insert(KeyCode::BTN_MIDDLE);
        let mut rel = AttributeSet::<Rel>::new();
        rel.insert(Rel::REL_X);
        rel.insert(Rel::REL_Y);
        rel.insert(Rel::REL_WHEEL);
        rel.insert(Rel::REL_WHEEL_HI_RES);
        let mouse = VirtualDevice::builder()?
            .name(&format!("{OWN_PREFIX} mouse"))
            .with_keys(&buttons)?
            .with_relative_axes(&rel)?
            .build()?;
        // O compositor leva um instante para adotar aparelhos novos.
        thread::sleep(Duration::from_millis(200));
        Ok(Out {
            keyboard,
            mouse,
            rest_x: 0.0,
            rest_y: 0.0,
            rest_wheel: 0.0,
        })
    }

    fn key_ev(k: KeyCode, down: bool) -> InputEvent {
        InputEvent::new(EventType::KEY.0, k.code(), i32::from(down))
    }

    /// Aperta as teclas na ordem e solta na ordem contrária ("ctrl+shift+[").
    pub fn keys(&mut self, keys: &[KeyCode]) -> Result<()> {
        for &k in keys {
            self.keyboard.emit(&[Self::key_ev(k, true)])?;
        }
        thread::sleep(Duration::from_millis(12));
        for &k in keys.iter().rev() {
            self.keyboard.emit(&[Self::key_ev(k, false)])?;
        }
        Ok(())
    }

    pub fn click(&mut self, c: Click, down: bool) -> Result<()> {
        let k = match c {
            Click::Left => KeyCode::BTN_LEFT,
            Click::Right => KeyCode::BTN_RIGHT,
            Click::Middle => KeyCode::BTN_MIDDLE,
        };
        self.mouse.emit(&[Self::key_ev(k, down)])?;
        Ok(())
    }

    /// Move o mouse; aceita frações de pixel (guardadas para o próximo).
    pub fn mouse_move(&mut self, dx: f64, dy: f64) -> Result<()> {
        self.rest_x += dx;
        self.rest_y += dy;
        let (x, y) = (self.rest_x.trunc(), self.rest_y.trunc());
        self.rest_x -= x;
        self.rest_y -= y;
        let mut evs = Vec::with_capacity(2);
        if x != 0.0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE.0,
                Rel::REL_X.0,
                x as i32,
            ));
        }
        if y != 0.0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE.0,
                Rel::REL_Y.0,
                y as i32,
            ));
        }
        if !evs.is_empty() {
            self.mouse.emit(&evs)?;
        }
        Ok(())
    }

    /// Rola `lines` linhas (positivo = para cima), em frações.
    pub fn scroll(&mut self, lines: f64) -> Result<()> {
        let before = (self.rest_wheel / WHEEL_HI).trunc();
        let hi = (lines * WHEEL_HI).round();
        if hi == 0.0 {
            return Ok(());
        }
        self.rest_wheel += hi;
        let after = (self.rest_wheel / WHEEL_HI).trunc();
        let mut evs = vec![InputEvent::new(
            EventType::RELATIVE.0,
            Rel::REL_WHEEL_HI_RES.0,
            hi as i32,
        )];
        let whole = (after - before) as i32;
        if whole != 0 {
            evs.push(InputEvent::new(
                EventType::RELATIVE.0,
                Rel::REL_WHEEL.0,
                whole,
            ));
        }
        // Não deixa a conta crescer para sempre.
        if self.rest_wheel.abs() > 1e6 {
            self.rest_wheel %= WHEEL_HI;
        }
        self.mouse.emit(&evs)?;
        Ok(())
    }

    pub fn reset_motion(&mut self) {
        self.rest_x = 0.0;
        self.rest_y = 0.0;
    }
}
