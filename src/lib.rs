mod addons;
mod ble;
mod discord;
mod perf;
mod ride;
pub mod android_data_dir;
pub mod android_dualscreen;
pub mod android_pip;

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod linux_runtime;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod rpc;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod lib_linux;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod lib_tauri;

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub use lib_linux::run;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use lib_tauri::run;


