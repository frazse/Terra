mod addons;
mod ble;

#[cfg(target_os = "linux")]
mod linux_runtime;
#[cfg(target_os = "linux")]
mod rpc;
#[cfg(target_os = "linux")]
mod lib_linux;
#[cfg(not(target_os = "linux"))]
mod lib_tauri;

#[cfg(target_os = "linux")]
pub use lib_linux::run;
#[cfg(not(target_os = "linux"))]
pub use lib_tauri::run;
