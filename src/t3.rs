//! Traz a janela do T3 Code para a frente, pelo COSMIC.
//!
//! Uma thread acompanha as janelas (`zcosmic_toplevel_info_v1`) e lembra qual
//! janela do T3 esteve em foco por último; `focus()` pede ao COSMIC para
//! ativá-la (`zcosmic_toplevel_manager_v1`), mesmo minimizada. O T3 volta com
//! o cursor onde estava: na caixinha da conversa.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result, anyhow};
use cosmic_protocols::toplevel_info::v1::client::{zcosmic_toplevel_handle_v1, zcosmic_toplevel_info_v1};
use cosmic_protocols::toplevel_management::v1::client::zcosmic_toplevel_manager_v1;
use wayland_client::backend::ObjectId;
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, event_created_child};

/// O app id do T3 Code (o AppImage).
const T3_APP_ID: &str = "com.t3tools.T3Code";
const ACTIVATED: u32 = 2;

#[derive(Default)]
struct Window {
    handle: Option<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1>,
    app_id: String,
    active: bool,
    /// Quando ficou ativa pela última vez (contador); 0 = nunca.
    last_active: u64,
}

#[derive(Default)]
struct Shared {
    windows: HashMap<ObjectId, Window>,
    tick: u64,
    manager: Option<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1>,
    seat: Option<wl_seat::WlSeat>,
}

struct State {
    shared: Arc<Mutex<Shared>>,
    info: Option<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1>,
}

pub struct T3 {
    conn: Connection,
    shared: Arc<Mutex<Shared>>,
}

impl T3 {
    /// Conecta ao COSMIC e sobe a thread que acompanha as janelas.
    pub fn connect() -> Result<T3> {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return Err(anyhow!("sem WAYLAND_DISPLAY (o pad precisa rodar dentro da sessão)"));
        }
        let conn = Connection::connect_to_env().context("sem conexão com o Wayland")?;
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let _registry = conn.display().get_registry(&qh, ());
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut state = State { shared: shared.clone(), info: None };
        queue.roundtrip(&mut state)?;
        {
            let s = shared.lock().unwrap();
            if state.info.is_none() || s.manager.is_none() {
                return Err(anyhow!("o compositor não tem os protocolos de janela do COSMIC"));
            }
        }
        queue.roundtrip(&mut state)?;
        queue.roundtrip(&mut state)?;
        {
            let s = shared.lock().unwrap();
            let n = s.windows.values().filter(|w| w.app_id == T3_APP_ID).count();
            eprintln!("[t3] {} janelas, {n} do T3", s.windows.len());
        }
        thread::Builder::new().name("t3".into()).spawn(move || {
            loop {
                if let Err(e) = queue.blocking_dispatch(&mut state) {
                    eprintln!("[t3] o Wayland caiu: {e}");
                    return;
                }
            }
        })?;
        Ok(T3 { conn, shared })
    }

    /// A janela do T3 já está em foco?
    pub fn is_focused(&self) -> bool {
        let s = self.shared.lock().unwrap();
        s.windows.values().any(|w| w.active && w.app_id == T3_APP_ID)
    }

    /// Ativa a última janela do T3 em foco. `Ok(false)`: o T3 não está aberto.
    pub fn focus(&self) -> Result<bool> {
        let s = self.shared.lock().unwrap();
        let (Some(manager), Some(seat)) = (&s.manager, &s.seat) else { return Ok(false) };
        let Some(w) = s
            .windows
            .values()
            .filter(|w| w.app_id == T3_APP_ID && w.handle.is_some())
            .max_by_key(|w| w.last_active)
        else {
            return Ok(false);
        };
        if w.active {
            return Ok(true);
        }
        let handle = w.handle.as_ref().unwrap();
        manager.unset_minimized(handle);
        manager.activate(handle, seat);
        drop(s);
        self.conn.flush().context("falando com o COSMIC")?;
        Ok(true)
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global { name, interface, version } = event else { return };
        let mut s = state.shared.lock().unwrap();
        match interface.as_str() {
            "zcosmic_toplevel_info_v1" => state.info = Some(registry.bind(name, 1, qh, ())),
            "zcosmic_toplevel_manager_v1" => s.manager = Some(registry.bind(name, version.min(2), qh, ())),
            "wl_seat" if s.seat.is_none() => s.seat = Some(registry.bind(name, 1, qh, ())),
            _ => {}
        }
    }
}

impl Dispatch<zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1,
        event: zcosmic_toplevel_info_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zcosmic_toplevel_info_v1::Event::Toplevel { toplevel } = event {
            let mut s = state.shared.lock().unwrap();
            s.windows.insert(toplevel.id(), Window { handle: Some(toplevel), ..Window::default() });
        }
    }

    event_created_child!(State, zcosmic_toplevel_info_v1::ZcosmicToplevelInfoV1, [
        zcosmic_toplevel_info_v1::EVT_TOPLEVEL_OPCODE => (zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()),
    ]);
}

impl Dispatch<zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &zcosmic_toplevel_handle_v1::ZcosmicToplevelHandleV1,
        event: zcosmic_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zcosmic_toplevel_handle_v1::Event;
        let mut s = state.shared.lock().unwrap();
        let id = handle.id();
        match event {
            Event::AppId { app_id } => s.windows.entry(id).or_default().app_id = app_id,
            Event::State { state: raw } => {
                let active = raw.as_chunks::<4>().0.iter().any(|c| u32::from_ne_bytes(*c) == ACTIVATED);
                s.tick += 1;
                let tick = s.tick;
                let w = s.windows.entry(id).or_default();
                if active && !w.active {
                    w.last_active = tick;
                }
                w.active = active;
            }
            Event::Closed => {
                s.windows.remove(&id);
                handle.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &zcosmic_toplevel_manager_v1::ZcosmicToplevelManagerV1,
        _: zcosmic_toplevel_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
