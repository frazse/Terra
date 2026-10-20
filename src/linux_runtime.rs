//! Lightweight Tauri replacement for Linux RPC mode.
//!
//! On Linux the binary runs only as an RPC backend (`--rpc`). Electron handles the webview;
//! we only need state management, path resolution, and an async runtime.
//! This module replaces Tauri's `AppHandle`, `State`, and `async_runtime`
//! without pulling in webkit2gtk / GTK.

use std::marker::PhantomData;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------- state container ----------

/// Generic state holder. Mirrors Tauri's `State<'_, T>` signature for compatibility.
/// The lifetime is unused (our state is owned, not borrowed) but keeps signatures compatible.
pub struct State<'a, T: Clone + Send + Sync> {
    inner: T,
    _phantom: PhantomData<&'a ()>,
}

impl<'a, T: Clone + Send + Sync> State<'a, T> {
    pub fn into_inner(self) -> T {
        self.inner
    }
}

impl<'a, T: Clone + Send + Sync> std::ops::Deref for State<'a, T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

/// Stores typed state entries.
#[derive(Default)]
pub struct StateMap {
    entries: std::sync::RwLock<Vec<(std::any::TypeId, Box<dyn std::any::Any + Send + Sync>)>>,
}

impl StateMap {
    pub fn insert<T: Clone + Send + Sync + 'static>(&self, value: T) {
        let mut g = self.entries.write().unwrap();
        g.retain(|(id, _)| *id != std::any::TypeId::of::<T>());
        g.push((std::any::TypeId::of::<T>(), Box::new(value)));
    }

    pub fn get<T: Clone + Send + Sync + 'static>(&self) -> Option<T> {
        let g = self.entries.read().unwrap();
        let (_, box_any) = g.iter().find(|(id, _)| *id == std::any::TypeId::of::<T>())?;
        box_any.downcast_ref::<T>().cloned()
    }
}

// ---------- AppHandle ----------

#[derive(Clone)]
pub struct AppHandle {
    states: std::sync::Arc<StateMap>,
    version: String,
}

impl AppHandle {
    pub fn new() -> Self {
        Self {
            states: std::sync::Arc::new(StateMap::default()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    /// Clone self (replaces `app.handle()` from Tauri's Manager trait).
    pub fn handle(&self) -> AppHandle {
        Self {
            states: self.states.clone(),
            version: self.version.clone(),
        }
    }

    /// Store a state value.
    pub fn manage<T: Clone + Send + Sync + 'static>(&self, value: T) {
        self.states.insert(value);
    }

    /// Retrieve a state value.
    pub fn state<'a, T>(&'a self) -> State<'a, T>
    where
        T: Clone + Send + Sync + 'static,
    {
        State {
            inner: self.states.get::<T>().expect("state not registered"),
            _phantom: PhantomData,
        }
    }

    /// Try to retrieve a state value.
    pub fn try_state<'a, T>(&'a self) -> Option<State<'a, T>>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.states.get::<T>().map(|inner| State {
            inner,
            _phantom: PhantomData,
        })
    }

    pub fn package_info(&self) -> &PackageInfo {
        &PACKAGE_INFO
    }

    pub fn path(&self) -> &PathProvider {
        &PATH_PROVIDER
    }

    /// Always `None` on Linux (no webview).
    pub fn get_webview_window(&self, _label: &str) -> Option<WebviewWindow> {
        None
    }
}

// ---------- PackageInfo ----------

#[derive(Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
    pub description: String,
}

static PACKAGE_INFO: std::sync::LazyLock<PackageInfo> = std::sync::LazyLock::new(|| PackageInfo {
    name: env!("CARGO_PKG_NAME").to_string(),
    version: env!("CARGO_PKG_VERSION").to_string(),
    description: env!("CARGO_PKG_DESCRIPTION").to_string(),
});

// ---------- Path helpers ----------

pub fn xdg_home() -> PathBuf {
    std::env::var("HOME").map(PathBuf::from).unwrap_or_default()
}

pub fn xdg_data_home() -> PathBuf {
    std::env::var("XDG_DATA_HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| Some(xdg_home().join(".local/share")))
        .unwrap_or_default()
}

fn xdg_download_dir() -> PathBuf {
    std::env::var("XDG_DOWNLOAD_DIR")
        .ok()
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| Some(xdg_home().join("Downloads")))
        .unwrap_or_default()
}

// ---------- PathProvider ----------

pub struct PathProvider;

static PATH_PROVIDER: std::sync::LazyLock<PathProvider> = std::sync::LazyLock::new(|| PathProvider);

impl PathProvider {
    pub fn app_data_dir(&self) -> Result<PathBuf, String> {
        Ok(xdg_data_home().join("terra"))
    }

    pub fn download_dir(&self) -> Result<PathBuf, String> {
        Ok(xdg_download_dir())
    }

    pub fn home_dir(&self) -> Result<PathBuf, String> {
        Ok(xdg_home())
    }
}

// ---------- WebviewWindow (stub) ----------

pub struct WebviewWindow;

impl WebviewWindow {
    pub fn eval(&self, _code: String) -> Result<(), String> {
        Err("no webview on Linux (RPC mode)".into())
    }

    pub fn close(&self) -> Result<(), String> {
        Ok(())
    }
}

// ---------- async_runtime ----------

pub mod async_runtime {
    pub use tokio::task::spawn;
    pub use tokio::task::JoinHandle;

    pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
            .block_on(future)
    }
}
