//! O ícone na bandeja: abrir os ajustes, ligar/desligar, sair.

use std::path::PathBuf;
use std::sync::mpsc::Sender;

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, ToolTip};

use crate::gamepad::Event;

pub use ksni::blocking::{Handle, TrayMethods};

pub struct Tray {
    pub enabled: bool,
    /// A vibração quando o T3 precisa de você ([notify].enabled).
    pub notify: bool,
    pub path: PathBuf,
    pub tx: Sender<Event>,
}

impl ksni::Tray for Tray {
    fn id(&self) -> String {
        "pad".into()
    }
    fn title(&self) -> String {
        "pad".into()
    }
    fn icon_name(&self) -> String {
        if self.enabled {
            "input-gaming-symbolic"
        } else {
            "action-unavailable-symbolic"
        }
        .into()
    }
    fn tool_tip(&self) -> ToolTip {
        let description = if self.enabled { "ligado" } else { "desligado" }.into();
        ToolTip {
            title: "pad".into(),
            description,
            ..Default::default()
        }
    }
    fn activate(&mut self, _x: i32, _y: i32) {
        crate::ui::open();
    }
    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Ajustes do pad".into(),
                activate: Box::new(|_: &mut Self| crate::ui::open()),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Ligado".into(),
                checked: self.enabled,
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Event::SetEnabled(!t.enabled));
                }),
                ..Default::default()
            }
            .into(),
            CheckmarkItem {
                label: "Vibrar quando o T3 precisar de mim".into(),
                checked: self.notify,
                activate: Box::new(|t: &mut Self| {
                    // Grava no config.toml; o pad relê e a marca acompanha.
                    if let Err(e) = crate::ui::set_flag(&t.path, "notify", "enabled", !t.notify) {
                        eprintln!("[pad] {e:#}");
                    }
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Sair".into(),
                activate: Box::new(|t: &mut Self| {
                    let _ = t.tx.send(Event::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}
