//! A tela de ajustes: uma página em http://127.0.0.1:7477, servida pelo pad.
//!
//! A página mostra o controle ao vivo (aperte um botão e ele fica marcado) e
//! grava no `config.toml` sem apagar os comentários; o pad relê sozinho.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde_json::{Value as Json, json};
use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::config::{self, Action, Button, Config};
use crate::gamepad::Event;
use crate::status::Shared;

pub const PORT: u16 = 7477;
const PAGE: &str = include_str!("ui.html");

pub fn url() -> String {
    format!("http://127.0.0.1:{PORT}/")
}

/// Abre a tela no navegador.
pub fn open() {
    let _ = std::process::Command::new("xdg-open")
        .arg(url())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| thread::spawn(move || c.wait()));
}

pub fn spawn(path: PathBuf, status: Shared, tx: Sender<Event>) {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ui] sem tela de ajustes (porta {PORT}): {e}");
            return;
        }
    };
    eprintln!("[ui] ajustes em {}", url());
    thread::Builder::new()
        .name("ui".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let (path, status, tx) = (path.clone(), status.clone(), tx.clone());
                thread::spawn(move || {
                    if let Err(e) = serve(stream, &path, &status, &tx) {
                        eprintln!("[ui] {e:#}");
                    }
                });
            }
        })
        .expect("thread da tela");
}

struct Request {
    method: String,
    path: String,
    query: String,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut r = BufReader::new(stream);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("/").to_string();
    let (path, query) = target
        .split_once('?')
        .map_or((target.clone(), String::new()), |(p, q)| {
            (p.into(), q.into())
        });
    let mut len = 0usize;
    let mut host_ok = false;
    let mut origin_ok = true;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        let Some((k, v)) = h.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "content-length" => len = v.parse().unwrap_or(0),
            // Só a própria página: nada de site de fora mexendo no pad.
            "host" => {
                host_ok = v == format!("127.0.0.1:{PORT}") || v == format!("localhost:{PORT}")
            }
            "origin" => {
                origin_ok = v == format!("http://127.0.0.1:{PORT}")
                    || v == format!("http://localhost:{PORT}")
            }
            _ => {}
        }
    }
    if !host_ok || !origin_ok {
        bail!("pedido de fora recusado");
    }
    if len > 1 << 20 {
        bail!("pedido grande demais");
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        query,
        body,
    })
}

fn respond(mut stream: TcpStream, code: u16, kind: &str, body: &[u8]) -> Result<()> {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    write!(
        stream,
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)?;
    Ok(())
}

fn serve(stream: TcpStream, path: &Path, status: &Shared, tx: &Sender<Event>) -> Result<()> {
    let req = match read_request(&stream) {
        Ok(r) => r,
        Err(e) => return respond(stream, 400, "text/plain", e.to_string().as_bytes()),
    };
    let json_ok =
        |stream, v: Json| respond(stream, 200, "application/json", v.to_string().as_bytes());
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(stream, 200, "text/html; charset=utf-8", PAGE.as_bytes()),
        ("GET", "/api/live") => {
            let mut s = status.lock().unwrap();
            s.learn(req.query.contains("learn=1"));
            let v = json!({
                "enabled": s.enabled,
                "controller": s.controller,
                "pressed": s.pressed.iter().map(|b| b.name()).collect::<Vec<_>>(),
            });
            drop(s);
            json_ok(stream, v)
        }
        ("GET", "/api/state") => json_ok(stream, state(path)),
        ("POST", "/api/enabled") => {
            let on: bool = serde_json::from_slice(&req.body)?;
            let _ = tx.send(Event::SetEnabled(on));
            json_ok(stream, json!({ "ok": true }))
        }
        ("POST", "/api/rumble") => {
            let _ = tx.send(Event::Attention(crate::watch::Attention::Test));
            json_ok(stream, json!({ "ok": true }))
        }
        ("POST", "/api/edit") => match edit(path, &req.body) {
            Ok(()) => json_ok(stream, json!({ "ok": true })),
            Err(e) => json_ok(stream, json!({ "ok": false, "error": format!("{e:#}") })),
        },
        _ => respond(stream, 404, "text/plain", b"nada aqui"),
    }
}

/// Tudo o que a tela mostra do arquivo (as ações como estão escritas).
fn state(path: &Path) -> Json {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let error = Config::parse(&text).err().map(|e| format!("{e:#}"));
    let doc: DocumentMut = text.parse().unwrap_or_default();
    let get_str = |t: &str, k: &str| {
        doc.get(t)
            .and_then(|x| x.get(k))
            .and_then(|x| x.as_str())
            .map(String::from)
    };
    let get_num = |t: &str, k: &str| {
        doc.get(t)
            .and_then(|x| x.get(k))
            .and_then(|x| x.as_float().or_else(|| x.as_integer().map(|i| i as f64)))
    };
    let get_bool = |t: &str, k: &str| doc.get(t).and_then(|x| x.get(k)).and_then(|x| x.as_bool());
    let buttons: serde_json::Map<String, Json> = Button::ALL
        .iter()
        .map(|b| {
            (
                b.name().to_string(),
                json!(get_str("buttons", b.name()).unwrap_or_else(|| "none".into())),
            )
        })
        .collect();
    let combos: Vec<Json> = doc
        .get("combos")
        .and_then(|c| c.as_table_like())
        .map(|t| {
            t.iter()
                .map(|(action, v)| {
                    let list: Vec<String> = v
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    json!({ "action": action, "buttons": list })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "path": path.display().to_string(),
        "error": error,
        "buttons": buttons,
        "combos": combos,
        "mouse": {
            "speed": get_num("mouse", "speed").unwrap_or(1500.0),
            "deadzone": get_num("mouse", "deadzone").unwrap_or(0.15),
            "curve": get_num("mouse", "curve").unwrap_or(2.2),
        },
        "scroll": { "speed": get_num("scroll", "speed").unwrap_or(25.0) },
        "notify": {
            "enabled": get_bool("notify", "enabled").unwrap_or(true),
            "finished": get_bool("notify", "finished").unwrap_or(true),
            "question": get_bool("notify", "question").unwrap_or(true),
            "pulses": get_num("notify", "pulses").unwrap_or(2.0),
            "strength": get_num("notify", "strength").unwrap_or(0.6),
            "length_ms": get_num("notify", "length_ms").unwrap_or(150.0),
        },
        "talk": get_str("talk", "command").unwrap_or_default(),
        "t3": t3_commands(),
    })
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Edit {
    Button {
        button: String,
        action: String,
    },
    Number {
        section: String,
        key: String,
        value: f64,
    },
    Combo {
        action: String,
        buttons: Vec<String>,
    },
    RemoveCombo {
        action: String,
    },
    Flag {
        section: String,
        key: String,
        value: bool,
    },
    Reset,
}

/// Uma mudança da tela no arquivo. Confere antes de gravar: o arquivo nunca
/// fica inválido.
fn edit(path: &Path, body: &[u8]) -> Result<()> {
    let e: Edit = serde_json::from_slice(body).context("pedido inválido")?;
    let text = if matches!(e, Edit::Reset) {
        config::DEFAULT.to_string()
    } else {
        let mut doc: DocumentMut = std::fs::read_to_string(path)?.parse()?;
        match e {
            Edit::Button { button, action } => {
                let b = Button::parse(&button)?;
                Action::parse(&action)?;
                set_str(table(&mut doc, "buttons"), b.name(), &action);
            }
            Edit::Number {
                section,
                key,
                value: v,
            } => {
                let ok = matches!(
                    (section.as_str(), key.as_str()),
                    ("mouse", "speed" | "deadzone" | "curve")
                        | ("scroll", "speed")
                        | ("notify", "pulses" | "strength" | "length_ms")
                );
                if !ok || !v.is_finite() {
                    bail!("ajuste desconhecido: {section}.{key}");
                }
                let t = table(&mut doc, &section);
                let decor = t
                    .get(&key)
                    .and_then(|i| i.as_value())
                    .map(|v| v.decor().clone());
                // Contagens e milissegundos são inteiros no arquivo.
                let mut nv = if matches!(key.as_str(), "pulses" | "length_ms") {
                    toml_edit::Value::from(v.round() as i64)
                } else {
                    toml_edit::Value::from((v * 1000.0).round() / 1000.0)
                };
                if let Some(d) = decor {
                    *nv.decor_mut() = d;
                }
                t.insert(&key, Item::Value(nv));
            }
            Edit::Combo { action, buttons } => {
                Action::parse(&action)?;
                let mut arr = Array::new();
                for b in &buttons {
                    arr.push(Button::parse(b)?.name());
                }
                if arr.len() < 2 {
                    bail!("um combo precisa de pelo menos 2 botões");
                }
                table(&mut doc, "combos").insert(&action, value(arr));
            }
            Edit::RemoveCombo { action } => {
                table(&mut doc, "combos").remove(&action);
            }
            Edit::Flag {
                section,
                key,
                value: on,
            } => {
                let ok = matches!(
                    (section.as_str(), key.as_str()),
                    ("notify", "enabled" | "finished" | "question")
                );
                if !ok {
                    bail!("ajuste desconhecido: {section}.{key}");
                }
                let t = table(&mut doc, &section);
                let decor = t
                    .get(&key)
                    .and_then(|i| i.as_value())
                    .map(|v| v.decor().clone());
                let mut nv = toml_edit::Value::from(on);
                if let Some(d) = decor {
                    *nv.decor_mut() = d;
                }
                t.insert(&key, Item::Value(nv));
            }
            Edit::Reset => unreachable!(),
        }
        doc.to_string()
    };
    Config::parse(&text)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Liga/desliga uma chave do arquivo (a bandeja usa).
pub fn set_flag(path: &Path, section: &str, key: &str, on: bool) -> Result<()> {
    let body = json!({ "op": "flag", "section": section, "key": key, "value": on });
    edit(path, body.to_string().as_bytes())
}

fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    if !doc.contains_table(name) {
        doc.insert(name, Item::Table(Table::new()));
    }
    doc[name].as_table_mut().expect("é tabela")
}

/// Troca o valor mantendo o comentário do fim da linha.
fn set_str(t: &mut Table, key: &str, v: &str) {
    let decor = t
        .get(key)
        .and_then(|i| i.as_value())
        .map(|v| v.decor().clone());
    let mut nv = toml_edit::Value::from(v);
    if let Some(d) = decor {
        *nv.decor_mut() = d;
    }
    t.insert(key, Item::Value(nv));
}

/// Os atalhos do T3 (do keybindings.json dele) que valem com a janela em
/// foco, já no formato do pad ("ctrl+shift+[").
fn t3_commands() -> Vec<Json> {
    let Some(home) = std::env::var_os("HOME") else {
        return vec![];
    };
    let file = PathBuf::from(home).join(".t3/userdata/keybindings.json");
    let Ok(text) = std::fs::read_to_string(file) else {
        return vec![];
    };
    let Ok(list) = serde_json::from_str::<Vec<Json>>(&text) else {
        return vec![];
    };
    let mut seen = std::collections::HashSet::new();
    list.iter()
        .filter_map(|b| {
            let key = b.get("key")?.as_str()?;
            let cmd = b.get("command")?.as_str()?;
            let when = b.get("when").and_then(|w| w.as_str()).unwrap_or("");
            // Só o que vale no uso normal (não "com o terminal em foco" etc.).
            let usable = when
                .split("&&")
                .map(str::trim)
                .all(|w| w.is_empty() || w.starts_with('!') || w == "isDesktop");
            if !usable || !seen.insert(cmd.to_string()) {
                return None;
            }
            let keys = pad_keys(key)?;
            Some(json!({ "command": cmd, "label": label(cmd), "keys": keys }))
        })
        .collect()
}

/// "mod+shift+arrowup" → "ctrl+shift+up".
fn pad_keys(t3: &str) -> Option<String> {
    let mut out = Vec::new();
    let mut parts: Vec<&str> = t3.split('+').collect();
    if t3.ends_with("++") {
        parts.truncate(parts.len() - 2);
        parts.push("+");
    }
    for p in parts {
        let k = match p.to_ascii_lowercase().as_str() {
            "mod" | "ctrl" | "control" => "ctrl".to_string(),
            "shift" => "shift".into(),
            "alt" => "alt".into(),
            "meta" | "super" => "super".into(),
            "arrowup" => "up".into(),
            "arrowdown" => "down".into(),
            "arrowleft" => "left".into(),
            "arrowright" => "right".into(),
            "escape" => "esc".into(),
            other => other.to_string(),
        };
        out.push(k);
    }
    let s = out.join("+");
    config::parse_keys(&s).ok().map(|_| s)
}

fn label(cmd: &str) -> String {
    let known = [
        ("sidebar.toggle", "Mostrar/esconder a barra lateral"),
        ("navigation.back", "Voltar"),
        ("navigation.forward", "Avançar"),
        ("terminal.toggle", "Mostrar/esconder o terminal"),
        ("rightPanel.toggle", "Mostrar/esconder o painel direito"),
        (
            "threadPanel.toggle",
            "Mostrar/esconder o painel da conversa",
        ),
        ("composer.dictation", "Ditado do próprio T3"),
        ("rightPanel.close", "Fechar o painel"),
        ("view.reopenClosed", "Reabrir o que fechou"),
        ("diff.toggle", "Mostrar/esconder as mudanças (diff)"),
        ("preview.toggle", "Mostrar/esconder o preview"),
        ("commandPalette.toggle", "Paleta de comandos"),
        ("filePicker.toggle", "Abrir arquivo"),
        ("projectSearch.toggle", "Buscar no projeto"),
        ("usage.open", "Uso e custos"),
        ("theme.select", "Trocar o tema"),
        ("composer.sendAndNewThread", "Enviar e abrir nova conversa"),
        ("chat.new", "Nova conversa"),
        ("chat.newLocal", "Nova conversa local"),
        ("chat.newWithoutProject", "Nova conversa sem projeto"),
        ("chat.find", "Buscar na conversa"),
        ("modelPicker.toggle", "Escolher o modelo"),
        ("composer.host", "Escolher a máquina"),
        ("composer.effort", "Escolher o esforço"),
        ("composer.mode", "Trocar o modo"),
        ("composer.workspace", "Escolher o workspace"),
        ("composer.branch", "Escolher a branch"),
        ("composer.previousWorktree", "Worktree anterior"),
        ("pullRequest.copyNumber", "Copiar o número do PR"),
        ("editor.openFavorite", "Abrir no editor"),
        ("thread.previous", "Conversa anterior"),
        ("thread.next", "Próxima conversa"),
        ("thread.copyReference", "Copiar referência da conversa"),
        ("thread.settle", "Arquivar a conversa"),
        ("thread.pin", "Fixar a conversa"),
        ("thread.undo", "Desfazer"),
        ("workspace.focusNext", "Próxima área"),
        ("workspace.focusPrevious", "Área anterior"),
        ("composer.sendAlternate", "Enviar (alternativo)"),
        ("composer.sendBackground", "Enviar em segundo plano"),
        ("composer.stash", "Guardar o rascunho"),
        ("thread.editQueuedMessage", "Editar mensagem na fila"),
        (
            "thread.steerQueuedMessage",
            "Redirecionar com a mensagem na fila",
        ),
    ];
    if let Some((_, l)) = known.iter().find(|(c, _)| *c == cmd) {
        return (*l).to_string();
    }
    if let Some(n) = cmd.strip_prefix("thread.jump.") {
        return format!("Ir para a conversa {n}");
    }
    cmd.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn t3_keys() {
        assert_eq!(pad_keys("mod+shift+[").as_deref(), Some("ctrl+shift+["));
        assert_eq!(
            pad_keys("mod+shift+arrowup").as_deref(),
            Some("ctrl+shift+up")
        );
        assert_eq!(pad_keys("mod++").as_deref(), Some("ctrl++"));
    }

    #[test]
    fn edit_keeps_comments_and_validates() {
        let dir = std::env::temp_dir().join(format!("pad-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("config.toml");
        std::fs::write(&p, config::DEFAULT).unwrap();
        edit(&p, br#"{"op":"button","button":"x","action":"t3 ctrl+n"}"#).unwrap();
        edit(&p, br#"{"op":"button","button":"up","action":"scroll up"}"#).unwrap();
        edit(
            &p,
            br#"{"op":"number","section":"mouse","key":"speed","value":2000}"#,
        )
        .unwrap();
        edit(
            &p,
            br#"{"op":"combo","action":"t3 ctrl+n","buttons":["l1","r1"]}"#,
        )
        .unwrap();
        assert!(edit(&p, br#"{"op":"button","button":"x","action":"banana"}"#).is_err());
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("x = \"t3 ctrl+n\""));
        assert!(text.contains("up = \"scroll up\"     # conversa anterior"));
        assert!(text.contains("speed = 2000.0"));
        assert!(text.contains("# Ações (o texto de cada botão):"));
        let c = Config::parse(&text).unwrap();
        assert_eq!(c.combos.len(), 2);
        edit(&p, br#"{"op":"remove_combo","action":"t3 ctrl+n"}"#).unwrap();
        assert_eq!(
            Config::parse(&std::fs::read_to_string(&p).unwrap())
                .unwrap()
                .combos
                .len(),
            1
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
