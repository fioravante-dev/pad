//! O vigia do T3: avisa quando uma resposta termina ou quando o T3 pergunta
//! algo e espera você.
//!
//! Lê o banco do próprio T3 (`~/.t3/userdata/statev2.sqlite`, só leitura) a
//! cada 2 s. Conta só as conversas de cima: os subagentes terminando não
//! chamam. Respostas canceladas ou interrompidas (por você) também não.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::gamepad::Event;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attention {
    /// Uma resposta terminou (ou falhou).
    Finished,
    /// O T3 fez uma pergunta e está esperando.
    Question,
    /// O botão "testar" da tela.
    Test,
}

const EVERY: Duration = Duration::from_secs(2);

fn db_path() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".t3/userdata/statev2.sqlite"))
}

/// Até onde já olhou: o maior horário visto de cada tabela.
struct Marks {
    run: String,
    request: String,
}

pub fn spawn(tx: Sender<Event>) {
    let Some(path) = db_path() else { return };
    thread::Builder::new()
        .name("t3-watch".into())
        .spawn(move || {
            let mut conn: Option<Connection> = None;
            let mut marks: Option<Marks> = None;
            let mut warned = false;
            loop {
                if conn.is_none() && path.exists() {
                    conn = Connection::open_with_flags(
                        &path,
                        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
                    )
                    .ok();
                }
                if let Some(c) = &conn {
                    let r = match &mut marks {
                        // Na primeira vez só marca onde está: o que já passou não chama.
                        None => start(c).map(|m| {
                            marks = Some(m);
                            None
                        }),
                        Some(m) => poll(c, m),
                    };
                    match r {
                        Ok(Some(a)) => {
                            if tx.send(Event::Attention(a)).is_err() {
                                return;
                            }
                        }
                        Ok(None) => warned = false,
                        Err(e) => {
                            if !warned {
                                eprintln!("[pad] vigia do T3: {e}");
                                warned = true;
                            }
                            // O T3 pode ter trocado o banco: abre de novo na próxima.
                            conn = None;
                        }
                    }
                }
                thread::sleep(EVERY);
            }
        })
        .expect("thread do vigia");
}

fn start(c: &Connection) -> rusqlite::Result<Marks> {
    let run = c.query_row(
        "SELECT coalesce(max(completed_at), '') FROM orchestration_v2_projection_runs",
        [],
        |r| r.get(0),
    )?;
    let request = c.query_row(
        "SELECT coalesce(max(created_at), '') FROM orchestration_v2_projection_runtime_requests",
        [],
        |r| r.get(0),
    )?;
    Ok(Marks { run, request })
}

fn poll(c: &Connection, m: &mut Marks) -> rusqlite::Result<Option<Attention>> {
    let mut finished = false;
    let mut s = c.prepare_cached(
        "SELECT r.completed_at, r.status,
                json_extract(t.payload_json, '$.lineage.parentThreadId') IS NULL
           FROM orchestration_v2_projection_runs r
           LEFT JOIN orchestration_v2_projection_threads t ON t.thread_id = r.thread_id
          WHERE r.completed_at > ?1
          ORDER BY r.completed_at",
    )?;
    let rows = s.query_map([&m.run], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<bool>>(2)?))
    })?;
    for row in rows {
        let (at, status, top) = row?;
        m.run = at;
        if matches!(status.as_str(), "completed" | "failed") && top.unwrap_or(true) {
            finished = true;
        }
    }

    let mut question = false;
    let mut s = c.prepare_cached(
        "SELECT created_at, status FROM orchestration_v2_projection_runtime_requests
          WHERE created_at > ?1 ORDER BY created_at",
    )?;
    let rows = s.query_map([&m.request], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (at, status) = row?;
        m.request = at;
        if !matches!(status.as_str(), "resolved" | "cancelled" | "expired") {
            question = true;
        }
    }

    Ok(if question {
        Some(Attention::Question)
    } else if finished {
        Some(Attention::Finished)
    } else {
        None
    })
}
