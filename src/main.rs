//! pad: o controle (8BitDo, Xbox...) como controle remoto do T3 Code.
//!
//! Lê o controle (evdev), aperta teclas e move o mouse por aparelhos virtuais
//! (uinput) e traz o T3 para a frente pelo COSMIC. Configuração em
//! `~/.config/pad/config.toml`, relida sozinha.

mod config;
mod engine;
mod gamepad;
mod out;
mod status;
mod t3;
mod tray;
mod ui;
mod watch;

use std::os::linux::net::SocketAddrExt;
use std::os::unix::net::{SocketAddr, UnixListener};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use crate::config::Config;
use crate::engine::Engine;
use crate::tray::TrayMethods as _;

fn main() {
    if let Err(e) = run() {
        eprintln!("[pad] erro: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let path = config::path();
    match std::env::args().nth(1).as_deref() {
        None | Some("run") => {}
        Some("check") => {
            Config::load(&path)?;
            println!("{}: ok", path.display());
            return Ok(());
        }
        Some("ui") => {
            ui::open();
            return Ok(());
        }
        Some("default-config") => {
            print!("{}", config::DEFAULT);
            return Ok(());
        }
        Some(other) => {
            eprintln!("uso: pad [run | ui | check | default-config]   (não conheço {other:?})");
            std::process::exit(2);
        }
    }

    // Um pad só por pessoa: dois apertariam tudo em dobro.
    let uid = std::fs::metadata("/proc/self")
        .map(|m| std::os::unix::fs::MetadataExt::uid(&m))
        .unwrap_or(0);
    let addr = SocketAddr::from_abstract_name(format!("pad-{uid}"))?;
    let _lock = UnixListener::bind_addr(&addr).context("o pad já está rodando")?;

    let cfg = Config::load(&path)?;
    eprintln!("[pad] config: {}", path.display());
    let out = out::Out::new()?;
    let t3 = match t3::T3::connect() {
        Ok(t) => Some(t),
        Err(e) => {
            eprintln!("[pad] sem trazer o T3 para a frente: {e:#}");
            None
        }
    };
    let (tx, rx) = mpsc::channel();
    gamepad::spawn(cfg.device.clone(), tx.clone());
    watch::spawn(tx.clone());
    let status = status::Shared::default();
    ui::spawn(path.clone(), status.clone(), tx.clone());
    let tray = match (tray::Tray {
        enabled: cfg.enabled,
        notify: cfg.notify.enabled,
        path: path.clone(),
        tx: tx.clone(),
    })
    .spawn()
    {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("[pad] sem ícone na bandeja: {e}");
            None
        }
    };

    let mut engine = Engine::new(cfg, out, t3, status, tray);
    let mut mtime = modified(&path);
    let mut last_check = Instant::now();
    let mut last_tick = Instant::now();
    loop {
        let wait = if engine.moving() {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(500)
        };
        match rx.recv_timeout(wait) {
            Ok(gamepad::Event::Quit) => {
                eprintln!("[pad] saindo");
                return Ok(());
            }
            Ok(ev) => engine.event(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("a thread do controle parou")
            }
        }
        let now = Instant::now();
        if now.duration_since(last_tick) >= Duration::from_millis(7) {
            let dt = now.duration_since(last_tick).as_secs_f64().min(0.05);
            engine.tick(dt);
            last_tick = now;
        }
        if now.duration_since(last_check) >= Duration::from_secs(1) {
            last_check = now;
            let m = modified(&path);
            if m != mtime {
                mtime = m;
                match Config::load(&path) {
                    Ok(c) => {
                        eprintln!("[pad] config relida");
                        engine.set_config(c);
                    }
                    Err(e) => {
                        eprintln!("[pad] config com erro, sigo com a anterior: {e:#}");
                        engine::notify(&format!("config com erro: {e:#}"));
                    }
                }
            }
        }
    }
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
