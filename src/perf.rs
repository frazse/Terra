//! Cross-platform hardware & process performance telemetry (`terra.perf.stats()`).
//!
//! Zero extra crate dependencies:
//! - Linux: `/proc/stat`, `/proc/meminfo`, `/sys/class/hwmon`, `/sys/class/drm`, optional cached `nvidia-smi`
//! - Android: JNI call to `MainActivity.getPerfStatsJson()` (PowerManager thermal status, battery temp, RAM, best-effort sysfs)
//! - Windows/macOS: optional cached `nvidia-smi` (Electron host merges Chromium `app.getAppMetrics()` + `os.*`)

#[cfg(not(target_os = "android"))]
use std::sync::{Arc, Mutex};
#[cfg(not(target_os = "android"))]
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
use tauri::State;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::linux_runtime::State;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PerfStats {
    pub platform: &'static str,
    pub cpu_pct: Option<f64>,
    pub cpu_temp_c: Option<f64>,
    pub app_cpu_pct: Option<f64>,
    pub gpu_pct: Option<f64>,
    pub gpu_temp_c: Option<f64>,
    pub gpu_vram_used_mb: Option<f64>,
    pub gpu_vram_total_mb: Option<f64>,
    pub mem_used_mb: Option<f64>,
    pub mem_total_mb: Option<f64>,
    pub thermal_status: Option<String>,
    pub battery_temp_c: Option<f64>,
}

#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone, Default)]
struct NvidiaSample {
    gpu_pct: Option<f64>,
    gpu_temp_c: Option<f64>,
    vram_used_mb: Option<f64>,
    vram_total_mb: Option<f64>,
}

#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone)]
struct NvidiaCache {
    fetched_at: Instant,
    ttl: Duration,
    sample: Option<NvidiaSample>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct CpuSample {
    total: u64,
    idle: u64,
}

#[derive(Clone)]
pub struct PerfState {
    #[cfg(target_os = "linux")]
    prev_cpu: Arc<Mutex<Option<CpuSample>>>,
    #[cfg(not(target_os = "android"))]
    nvidia_cache: Arc<Mutex<Option<NvidiaCache>>>,
}

impl Default for PerfState {
    fn default() -> Self {
        Self::new()
    }
}

impl PerfState {
    pub fn new() -> Self {
        #[cfg(target_os = "linux")]
        let initial_cpu = read_linux_cpu_sample();

        Self {
            #[cfg(target_os = "linux")]
            prev_cpu: Arc::new(Mutex::new(initial_cpu)),
            #[cfg(not(target_os = "android"))]
            nvidia_cache: Arc::new(Mutex::new(None)),
        }
    }

    pub fn sample(&self) -> PerfStats {
        #[cfg(target_os = "linux")]
        {
            return self.sample_linux();
        }
        #[cfg(target_os = "android")]
        {
            return self.sample_android();
        }
        #[cfg(all(not(target_os = "linux"), not(target_os = "android")))]
        {
            return self.sample_desktop_other();
        }
    }

    #[cfg(target_os = "linux")]
    fn sample_linux(&self) -> PerfStats {
        let mut out = PerfStats {
            platform: "linux",
            ..Default::default()
        };

        out.cpu_pct = self.sample_linux_cpu_pct();
        out.cpu_temp_c = read_linux_cpu_temp();

        let (mem_used, mem_total) = read_linux_meminfo();
        out.mem_used_mb = mem_used;
        out.mem_total_mb = mem_total;

        let drm = read_linux_drm_gpu();
        out.gpu_pct = drm.gpu_pct;
        out.gpu_temp_c = drm.gpu_temp_c.or_else(read_linux_hwmon_gpu_temp);
        out.gpu_vram_used_mb = drm.vram_used_mb;
        out.gpu_vram_total_mb = drm.vram_total_mb;

        if out.gpu_pct.is_none() {
            if let Some(nv) = self.query_nvidia_smi_cached() {
                out.gpu_pct = nv.gpu_pct;
                if out.gpu_temp_c.is_none() {
                    out.gpu_temp_c = nv.gpu_temp_c;
                }
                if out.gpu_vram_used_mb.is_none() {
                    out.gpu_vram_used_mb = nv.vram_used_mb;
                }
                if out.gpu_vram_total_mb.is_none() {
                    out.gpu_vram_total_mb = nv.vram_total_mb;
                }
            }
        }

        out
    }

    #[cfg(target_os = "linux")]
    fn sample_linux_cpu_pct(&self) -> Option<f64> {
        let cur = read_linux_cpu_sample()?;
        let mut guard = self.prev_cpu.lock().ok()?;
        let prev = *guard;
        *guard = Some(cur);
        let prev = prev?;
        let total_delta = cur.total.saturating_sub(prev.total);
        if total_delta == 0 {
            return None;
        }
        let idle_delta = cur.idle.saturating_sub(prev.idle).min(total_delta);
        let busy = (total_delta - idle_delta) as f64;
        let pct = (busy / total_delta as f64) * 100.0;
        Some(round1(pct.clamp(0.0, 100.0)))
    }

    #[cfg(target_os = "android")]
    fn sample_android(&self) -> PerfStats {
        let mut out = PerfStats {
            platform: "android",
            ..Default::default()
        };
        if let Ok(json_str) = crate::android_data_dir::call_main_activity_str_pub("getPerfStatsJson")
        {
            if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
                out.app_cpu_pct = val.get("appCpuPct").and_then(|v| v.as_f64()).map(round1);
                out.cpu_temp_c = val.get("cpuTempC").and_then(|v| v.as_f64()).map(round1);
                out.gpu_pct = val.get("gpuPct").and_then(|v| v.as_f64()).map(round1);
                out.gpu_temp_c = val.get("gpuTempC").and_then(|v| v.as_f64()).map(round1);
                out.mem_used_mb = val.get("memUsedMb").and_then(|v| v.as_f64()).map(round1);
                out.mem_total_mb = val.get("memTotalMb").and_then(|v| v.as_f64()).map(round1);
                out.thermal_status = val
                    .get("thermalStatus")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                out.battery_temp_c = val.get("batteryTempC").and_then(|v| v.as_f64()).map(round1);
            }
        }
        out
    }

    #[cfg(all(not(target_os = "linux"), not(target_os = "android")))]
    fn sample_desktop_other(&self) -> PerfStats {
        let mut out = PerfStats {
            platform: std::env::consts::OS,
            ..Default::default()
        };
        if let Some(nv) = self.query_nvidia_smi_cached() {
            out.gpu_pct = nv.gpu_pct;
            out.gpu_temp_c = nv.gpu_temp_c;
            out.gpu_vram_used_mb = nv.vram_used_mb;
            out.gpu_vram_total_mb = nv.vram_total_mb;
        }
        out
    }

    #[cfg(not(target_os = "android"))]
    fn query_nvidia_smi_cached(&self) -> Option<NvidiaSample> {
        let now = Instant::now();
        if let Ok(guard) = self.nvidia_cache.lock() {
            if let Some(ref c) = *guard {
                if now.duration_since(c.fetched_at) < c.ttl {
                    return c.sample.clone();
                }
            }
        }

        let fetched = run_nvidia_smi();
        let ttl = if fetched.is_some() {
            Duration::from_millis(850)
        } else {
            // Avoid spawning a failing subprocess every second on non-NVIDIA machines.
            Duration::from_secs(60)
        };
        if let Ok(mut guard) = self.nvidia_cache.lock() {
            *guard = Some(NvidiaCache {
                fetched_at: Instant::now(),
                ttl,
                sample: fetched.clone(),
            });
        }
        fetched
    }
}

#[cfg_attr(not(any(target_os = "linux", target_os = "windows")), tauri::command)]
pub async fn perf_stats(state: State<'_, PerfState>) -> Result<Value, String> {
    let sampler = (*state).clone();
    let stats = tokio::task::spawn_blocking(move || sampler.sample())
        .await
        .map_err(|e| format!("perf task failed: {e}"))?;
    serde_json::to_value(stats).map_err(|e| e.to_string())
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

#[cfg(not(target_os = "android"))]
fn run_nvidia_smi() -> Option<NvidiaSample> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args([
        "--query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total",
        "--format=csv,noheader,nounits",
    ]);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let output = cmd.output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut best: Option<NvidiaSample> = None;
    for line in stdout.lines() {
        let parts: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 4 {
            continue;
        }
        let sample = NvidiaSample {
            gpu_pct: parts[0].parse::<f64>().ok().map(round1),
            gpu_temp_c: parts[1].parse::<f64>().ok().map(round1),
            vram_used_mb: parts[2].parse::<f64>().ok().map(round1),
            vram_total_mb: parts[3].parse::<f64>().ok().map(round1),
        };
        let replace = match &best {
            None => true,
            Some(cur) => {
                sample.gpu_pct.unwrap_or(-1.0) > cur.gpu_pct.unwrap_or(-1.0)
                    || (sample.gpu_pct == cur.gpu_pct
                        && sample.vram_used_mb.unwrap_or(0.0) > cur.vram_used_mb.unwrap_or(0.0))
            }
        };
        if replace {
            best = Some(sample);
        }
    }
    best
}

// ─── Linux sysfs / procfs readers ────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn read_linux_cpu_sample() -> Option<CpuSample> {
    let txt = std::fs::read_to_string("/proc/stat").ok()?;
    let line = txt.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "cpu" {
        return None;
    }
    let cols: Vec<u64> = parts.filter_map(|p| p.parse::<u64>().ok()).collect();
    if cols.len() < 4 {
        return None;
    }
    let idle = cols[3].saturating_add(*cols.get(4).unwrap_or(&0));
    let total: u64 = cols.iter().copied().sum();
    Some(CpuSample { total, idle })
}

#[cfg(target_os = "linux")]
fn read_linux_meminfo() -> (Option<f64>, Option<f64>) {
    let Ok(txt) = std::fs::read_to_string("/proc/meminfo") else {
        return (None, None);
    };
    let mut total_kb: Option<u64> = None;
    let mut avail_kb: Option<u64> = None;
    let mut free_kb: Option<u64> = None;

    for line in txt.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = parse_kb(rest);
        } else if let Some(rest) = line.strip_prefix("MemFree:") {
            free_kb = parse_kb(rest);
        }
        if total_kb.is_some() && avail_kb.is_some() {
            break;
        }
    }

    let Some(total) = total_kb else {
        return (None, None);
    };
    let avail = avail_kb.or(free_kb).unwrap_or(0);
    let used = total.saturating_sub(avail);
    (
        Some(round1(used as f64 / 1024.0)),
        Some(round1(total as f64 / 1024.0)),
    )
}

#[cfg(target_os = "linux")]
fn parse_kb(rest: &str) -> Option<u64> {
    rest.split_whitespace().next()?.parse::<u64>().ok()
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[cfg(target_os = "linux")]
fn read_temp_millideg(path: &std::path::Path) -> Option<f64> {
    let raw = read_trimmed(path)?.parse::<i64>().ok()?;
    let c = raw as f64 / 1000.0;
    if (-20.0..=150.0).contains(&c) {
        Some(round1(c))
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn read_hwmon_best_temp(hwmon_dir: &std::path::Path, preferred_labels: &[&str]) -> Option<f64> {
    let entries = std::fs::read_dir(hwmon_dir).ok()?;
    let mut best: Option<(usize, f64)> = None;

    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("temp") || !fname.ends_with("_input") {
            continue;
        }
        let Some(temp_c) = read_temp_millideg(&entry.path()) else {
            continue;
        };
        let prefix = fname.trim_end_matches("_input");
        let label = read_trimmed(&hwmon_dir.join(format!("{prefix}_label")))
            .unwrap_or_default()
            .to_lowercase();

        let score = preferred_labels
            .iter()
            .position(|pat| label.contains(pat))
            .unwrap_or_else(|| {
                if prefix == "temp1" {
                    preferred_labels.len()
                } else {
                    preferred_labels.len() + 1
                }
            });

        if best.map_or(true, |(b_score, _)| score < b_score) {
            best = Some((score, temp_c));
        }
    }

    best.map(|(_, t)| t)
}

#[cfg(target_os = "linux")]
fn read_linux_cpu_temp() -> Option<f64> {
    let hwmon_root = std::path::Path::new("/sys/class/hwmon");
    let driver_priority = [
        "k10temp",
        "zenpower",
        "coretemp",
        "cpu_thermal",
        "soc_thermal",
        "x86_pkg_temp",
        "thinkpad",
        "dell_smm",
    ];
    let label_priority = ["tctl", "tdie", "package", "cpu", "core 0"];

    let mut best: Option<(usize, f64)> = None;
    if let Ok(entries) = std::fs::read_dir(hwmon_root) {
        for entry in entries.flatten() {
            let dir = entry.path();
            let Some(name) = read_trimmed(&dir.join("name")) else {
                continue;
            };
            let name_lc = name.to_lowercase();
            let Some(prio) = driver_priority.iter().position(|d| name_lc == *d) else {
                continue;
            };
            if let Some(temp) = read_hwmon_best_temp(&dir, &label_priority) {
                if best.map_or(true, |(bp, _)| prio < bp) {
                    best = Some((prio, temp));
                }
            }
        }
    }
    if let Some((_, temp)) = best {
        return Some(temp);
    }

    // Thermal zone fallback (common on ARM SBCs / laptops without dedicated hwmon names)
    let tz_root = std::path::Path::new("/sys/class/thermal");
    let mut tz_best: Option<(usize, f64)> = None;
    let tz_priority = ["x86_pkg_temp", "cpu", "soc", "tboard", "acpitz"];
    if let Ok(entries) = std::fs::read_dir(tz_root) {
        for entry in entries.flatten() {
            let dir = entry.path();
            let fname = entry.file_name().to_string_lossy().to_string();
            if !fname.starts_with("thermal_zone") {
                continue;
            }
            let Some(tname) = read_trimmed(&dir.join("type")).map(|s| s.to_lowercase()) else {
                continue;
            };
            let Some(prio) = tz_priority.iter().position(|p| tname.contains(p)) else {
                continue;
            };
            if let Some(temp) = read_temp_millideg(&dir.join("temp")) {
                if tz_best.map_or(true, |(bp, _)| prio < bp) {
                    tz_best = Some((prio, temp));
                }
            }
        }
    }
    tz_best.map(|(_, t)| t)
}

#[cfg(target_os = "linux")]
fn read_linux_hwmon_gpu_temp() -> Option<f64> {
    let hwmon_root = std::path::Path::new("/sys/class/hwmon");
    let drivers = ["amdgpu", "radeon", "nouveau", "xe", "i915", "mali", "panfrost"];
    let labels = ["edge", "junction", "gpu", "mem"];
    let entries = std::fs::read_dir(hwmon_root).ok()?;
    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(name) = read_trimmed(&dir.join("name")).map(|s| s.to_lowercase()) else {
            continue;
        };
        if drivers.iter().any(|d| name.contains(d)) {
            if let Some(temp) = read_hwmon_best_temp(&dir, &labels) {
                return Some(temp);
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
#[derive(Default)]
struct DrmGpuSample {
    gpu_pct: Option<f64>,
    gpu_temp_c: Option<f64>,
    vram_used_mb: Option<f64>,
    vram_total_mb: Option<f64>,
}

#[cfg(target_os = "linux")]
fn read_linux_drm_gpu() -> DrmGpuSample {
    let drm_root = std::path::Path::new("/sys/class/drm");
    let Ok(entries) = std::fs::read_dir(drm_root) else {
        return DrmGpuSample::default();
    };

    let mut best: Option<DrmGpuSample> = None;

    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().to_string();
        if !fname.starts_with("card") || fname.contains('-') {
            continue;
        }
        let dev = entry.path().join("device");
        if !dev.exists() {
            continue;
        }

        let gpu_pct = read_trimmed(&dev.join("gpu_busy_percent"))
            .and_then(|s| s.parse::<f64>().ok())
            .map(|v| round1(v.clamp(0.0, 100.0)));

        let vram_used_mb = read_trimmed(&dev.join("mem_info_vram_used"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|b| round1(b as f64 / (1024.0 * 1024.0)));

        let vram_total_mb = read_trimmed(&dev.join("mem_info_vram_total"))
            .and_then(|s| s.parse::<u64>().ok())
            .map(|b| round1(b as f64 / (1024.0 * 1024.0)));

        let mut gpu_temp_c = None;
        if let Ok(hwmons) = std::fs::read_dir(dev.join("hwmon")) {
            for hw in hwmons.flatten() {
                if let Some(t) = read_hwmon_best_temp(&hw.path(), &["edge", "junction", "gpu"]) {
                    gpu_temp_c = Some(t);
                    break;
                }
            }
        }

        if gpu_pct.is_none() && gpu_temp_c.is_none() && vram_used_mb.is_none() {
            continue;
        }

        let cand = DrmGpuSample {
            gpu_pct,
            gpu_temp_c,
            vram_used_mb,
            vram_total_mb,
        };

        let replace = match &best {
            None => true,
            Some(cur) => {
                let c_busy = cand.gpu_pct.unwrap_or(-1.0);
                let b_busy = cur.gpu_pct.unwrap_or(-1.0);
                if (c_busy - b_busy).abs() > 0.01 {
                    c_busy > b_busy
                } else {
                    cand.vram_used_mb.unwrap_or(0.0) > cur.vram_used_mb.unwrap_or(0.0)
                }
            }
        };

        if replace {
            best = Some(cand);
        }
    }

    best.unwrap_or_default()
}
