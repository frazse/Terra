//! `terra --tui`: terminal front-end for a running Terra (Linux).
//!
//! Renders a high-refresh-rate curved Road View (±50 m / ±100 m around focal/ego for live
//! drafting & attack watching), filled grade-colored elevation profile, rider table, and
//! telemetry charts while Electron runs Biketerra with 3D disabled or throttled.

mod state;
mod ui;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::sync::mpsc;
use std::time::Duration;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use serde_json::{json, Value};

use state::{App, Screen};

enum Msg {
    Connected(UnixStream),
    Line(Value),
    Disconnected(String),
}

/// Background thread: (re)connect to the backend socket and forward its JSON lines.
fn spawn_reader(tx: mpsc::Sender<Msg>) {
    std::thread::spawn(move || {
        let path = crate::tui_hub::socket_path();
        loop {
            match UnixStream::connect(&path) {
                Ok(stream) => {
                    let Ok(writer) = stream.try_clone() else { continue };
                    if tx.send(Msg::Connected(writer)).is_err() {
                        return;
                    }
                    for line in BufReader::new(stream).lines() {
                        let Ok(line) = line else { break };
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            if tx.send(Msg::Line(v)).is_err() {
                                return;
                            }
                        }
                    }
                    if tx.send(Msg::Disconnected("backend closed the connection".into())).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    let why = format!("waiting for Terra ({}: {e})", path.display());
                    if tx.send(Msg::Disconnected(why)).is_err() {
                        return;
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn send(app: &mut App, writer: &mut Option<UnixStream>, cmd: Value, ok: &str) {
    match writer.as_mut() {
        Some(w) => {
            let line = format!("{cmd}\n");
            if w.write_all(line.as_bytes()).and_then(|_| w.flush()).is_ok() {
                app.toast(ok);
            } else {
                app.toast("send failed (backend gone?)");
            }
        }
        None => app.toast("not connected"),
    }
}

fn on_key(app: &mut App, writer: &mut Option<UnixStream>, code: KeyCode, mods: KeyModifiers) {
    if app.show_help && !matches!(code, KeyCode::Char('q')) {
        app.show_help = false;
        return;
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => app.quit = true,
        KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => app.quit = true,
        KeyCode::Char('?') => app.show_help = true,
        KeyCode::Char('1') => app.screen = Screen::Ride,
        KeyCode::Char('2') => app.screen = Screen::Spectate,
        KeyCode::Char('a') | KeyCode::Char('0') => app.screen = Screen::Auto,
        KeyCode::Up | KeyCode::Char('k') => app.move_selection(-1),
        KeyCode::Down | KeyCode::Char('j') => app.move_selection(1),
        KeyCode::PageUp => app.move_selection(-10),
        KeyCode::PageDown => app.move_selection(10),
        KeyCode::Home => app.move_selection(isize::MIN / 2),
        KeyCode::End => app.move_selection(isize::MAX / 2),
        KeyCode::Enter => {
            if let Some(r) = app.selected_rider().cloned() {
                let msg = format!("focusing {}", r.name);
                send(
                    app,
                    writer,
                    json!({ "cmd": "focus_rider", "id": r.id, "name": r.name }),
                    &msg,
                );
            }
        }
        KeyCode::Char('m') => {
            app.road_view = app.road_view.next();
            app.toast(format!("view: {}", app.road_view.label()));
        }
        KeyCode::Char('g') => {
            let next = app.render_mode.next();
            app.render_mode = next;
            let msg = format!("3D render mode → {}", next.as_str());
            send(app, writer, json!({ "cmd": "render_mode", "mode": next.as_str() }), &msg);
        }
        KeyCode::Char('w') => send(app, writer, json!({ "cmd": "window", "action": "toggle" }), "toggled browser window"),
        KeyCode::Char('r') => send(app, writer, json!({ "cmd": "reload" }), "reloading page"),
        KeyCode::Char('h') => send(app, writer, json!({ "cmd": "navigate", "path": "/" }), "opening biketerra.com home"),
        _ => {}
    }
}

pub fn run() -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    spawn_reader(tx);

    let mut terminal = ratatui::init();
    let mut app = App::new();
    let mut writer: Option<UnixStream> = None;

    let result = (|| -> Result<(), String> {
        while !app.quit {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    Msg::Connected(w) => {
                        writer = Some(w);
                        app.connected = true;
                        app.status = "connected".into();
                    }
                    Msg::Line(v) => app.on_message(&v),
                    Msg::Disconnected(why) => {
                        writer = None;
                        app.connected = false;
                        app.status = why;
                    }
                }
            }

            terminal.draw(|f| ui::draw(f, &app)).map_err(|e| e.to_string())?;

            // ~30 fps redraw cadence so 20 Hz road & rider positions flow smoothly.
            if event::poll(Duration::from_millis(33)).map_err(|e| e.to_string())? {
                if let Event::Key(k) = event::read().map_err(|e| e.to_string())? {
                    if k.kind == KeyEventKind::Press {
                        on_key(&mut app, &mut writer, k.code, k.modifiers);
                    }
                }
            }
        }
        Ok(())
    })();

    ratatui::restore();
    result
}
