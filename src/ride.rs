//! Ride broadcast data: while riding or spectating, biketerra broadcasts the ride state on
//! `BroadcastChannel('biketerra-ride')` at ~2Hz (biketerra.com/help-center/live-broadcast-data).
//! The in-page shim relays every frame here; addons read it back through the `terra.ride` API.

use std::sync::{Arc, Mutex};

use serde_json::Value;
#[cfg(not(target_os = "linux"))]
use tauri::State;
#[cfg(target_os = "linux")]
use crate::linux_runtime::State;

/// How many frames to keep (about five minutes at 2Hz).
const MAX_HISTORY: usize = 600;

#[derive(Clone, Default)]
pub struct Ride {
    /// Most recent frame (kept across page navigations).
    latest: Arc<Mutex<Option<Value>>>,
    /// Recent frames, oldest first.
    history: Arc<Mutex<std::collections::VecDeque<Value>>>,
}

impl Ride {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores a frame from the page (latest + ring buffer).
    pub fn push(&self, data: Value) {
        if let Ok(mut g) = self.latest.lock() {
            *g = Some(data.clone());
        }
        if let Ok(mut g) = self.history.lock() {
            g.push_back(data);
            while g.len() > MAX_HISTORY {
                g.pop_front();
            }
        }
    }

    /// The most recent frame, if any has been seen.
    pub fn latest(&self) -> Option<Value> {
        self.latest.lock().ok().and_then(|g| g.clone())
    }

    /// Recent frames, oldest to newest; all kept frames unless a smaller `limit` is given.
    pub fn history(&self, limit: Option<usize>) -> Vec<Value> {
        let g = self.history.lock().unwrap_or_else(|p| p.into_inner());
        let n = limit.unwrap_or(g.len()).min(g.len());
        g.iter()
        .rev()
        .take(n)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
    }
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn ride_push(state: State<'_, Ride>, data: Value) {
    state.push(data);
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn ride_latest(state: State<'_, Ride>) -> Option<Value> {
    state.latest()
}

#[cfg_attr(not(target_os = "linux"), tauri::command)]
pub fn ride_history(state: State<'_, Ride>, limit: Option<usize>) -> Vec<Value> {
    state.history(limit)
}
