//! O que a tela de ajustes enxerga do pad: ligado, controle, botões apertados.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::Button;

#[derive(Default)]
pub struct Status {
    pub enabled: bool,
    pub controller: Option<String>,
    pub pressed: Vec<Button>,
    /// Até quando os botões só aparecem na tela, sem disparar nada
    /// (a tela renova enquanto está aberta com "pausar" marcado).
    learn_until: Option<Instant>,
}

pub type Shared = Arc<Mutex<Status>>;

impl Status {
    pub fn learning(&self) -> bool {
        self.learn_until.is_some_and(|t| Instant::now() < t)
    }

    pub fn learn(&mut self, on: bool) {
        self.learn_until = on.then(|| Instant::now() + Duration::from_millis(800));
    }

    pub fn press(&mut self, b: Button, down: bool) {
        self.pressed.retain(|x| *x != b);
        if down {
            self.pressed.push(b);
        }
    }
}
