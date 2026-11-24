const { app, BrowserWindow, dialog, ipcMain, Menu, net, powerSaveBlocker } = require('electron');
const os = require('os');
const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');

// ---------------------------------------------------------------------------
// Experimental Frame Generation (lsfg-vk / Lossless Scaling) configuration.
// Must run before app.whenReady() because Chromium requires XWayland + Vulkan
// switches (--ozone-platform=x11 --use-gl=angle --use-angle=vulkan) at startup
// for Vulkan swapchains to be presented through lsfg-vk on Linux Wayland.
// ---------------------------------------------------------------------------
const FRAMEGEN_MODES = new Set(['off', '2x', '3x', '4x', 'adaptive60', 'adaptive120']);

function getTerraDataDir() {
  if (process.platform === 'win32') {
    const base = process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local');
    return path.join(base, 'terra');
  }
  const base = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share');
  return path.join(base, 'terra');
}

function getFramegenConfigPath() {
  return path.join(getTerraDataDir(), 'framegen.json');
}

function getLsfgTomlPath() {
  return path.join(getTerraDataDir(), 'lsfg-vk.toml');
}

function detectDefaultDllPath() {
  if (process.platform !== 'linux') return '';
  const xdgData = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share');
  const dirs = [
    path.join(xdgData, 'Steam', 'steamapps', 'common', 'Lossless Scaling'),
    path.join(os.homedir(), '.steam', 'steam', 'steamapps', 'common', 'Lossless Scaling'),
    path.join(os.homedir(), '.var', 'app', 'com.valvesoftware.Steam', '.local', 'share', 'Steam', 'steamapps', 'common', 'Lossless Scaling')
  ];
  for (const dir of dirs) {
    for (const name of ['lsfg-vk.dll', 'Lossless.dll']) {
      const candidate = path.join(dir, name);
      try {
        if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
          return candidate;
        }
      } catch {}
    }
  }
  return '';
}

function detectLsfgLayerInstalled() {
  if (process.platform !== 'linux') return false;
  const xdgData = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share');
  const layerDirs = [
    '/usr/share/vulkan/implicit_layer.d',
    '/usr/local/share/vulkan/implicit_layer.d',
    '/etc/vulkan/implicit_layer.d',
    path.join(xdgData, 'vulkan', 'implicit_layer.d'),
    path.join(os.homedir(), '.config', 'vulkan', 'implicit_layer.d')
  ];
  for (const dir of layerDirs) {
    try {
      const entries = fs.readdirSync(dir);
      if (entries.some((f) => /lsfg/i.test(f) && f.endsWith('.json'))) {
        return true;
      }
    } catch {}
  }
  return fs.existsSync('/usr/lib/liblsfg-vk-layer.so') || fs.existsSync('/usr/lib64/liblsfg-vk-layer.so');
}

function normalizeFramegenConfig(raw) {
  const cfg = raw && typeof raw === 'object' ? raw : {};
  const mode = FRAMEGEN_MODES.has(cfg.mode) ? cfg.mode : 'off';
  let dllPath = typeof cfg.dllPath === 'string' ? cfg.dllPath.trim() : '';
  if (!dllPath) dllPath = detectDefaultDllPath();
  let flowScale = Number(cfg.flowScale);
  if (!Number.isFinite(flowScale)) flowScale = 85;
  flowScale = Math.max(0, Math.min(100, Math.round(flowScale)));
  const performanceMode = Boolean(cfg.performanceMode);
  const rideOnly = cfg.rideOnly === undefined ? true : Boolean(cfg.rideOnly);
  return { mode, dllPath, flowScale, performanceMode, rideOnly };
}

function readFramegenConfig() {
  const cfgPath = getFramegenConfigPath();
  try {
    if (fs.existsSync(cfgPath)) {
      const parsed = JSON.parse(fs.readFileSync(cfgPath, 'utf8'));
      return normalizeFramegenConfig(parsed);
    }
  } catch (e) {
    console.warn('[electron] Failed to read framegen.json:', e && e.message ? e.message : e);
  }
  return normalizeFramegenConfig({});
}

function isExistingFile(p) {
  if (!p) return false;
  try {
    return fs.existsSync(p) && fs.statSync(p).isFile();
  } catch {
    return false;
  }
}

function isAdaptiveMode(mode) {
  return mode === 'adaptive60' || mode === 'adaptive120';
}

function isFramegenActiveConfig(cfg) {
  return process.platform === 'linux' && cfg.mode !== 'off' && isExistingFile(cfg.dllPath);
}

function isRideOrSpectateUrl(rawUrl) {
  if (!rawUrl) return false;
  try {
    const u = new URL(rawUrl);
    return (u.hostname === 'biketerra.com' || u.hostname.endsWith('.biketerra.com')) &&
      /^\/(ride|spectate)(\/|$)/.test(u.pathname);
  } catch {
    return false;
  }
}

let lsfgAdaptiveSupported = null;
let onRideOrSpectatePage = false;
let lastWrittenLsfgToml = null;

function detectLsfgAdaptiveSupported() {
  if (lsfgAdaptiveSupported !== null) return lsfgAdaptiveSupported;
  if (process.platform !== 'linux') {
    lsfgAdaptiveSupported = false;
    return false;
  }
  const candidates = [
    '/usr/lib/liblsfg-vk-layer.so',
    '/usr/lib64/liblsfg-vk-layer.so',
    '/usr/local/lib/liblsfg-vk-layer.so'
  ];
  for (const p of candidates) {
    try {
      if (fs.existsSync(p)) {
        const buf = fs.readFileSync(p);
        lsfgAdaptiveSupported = buf.includes('LSFGVK_ADAPTIVE');
        return lsfgAdaptiveSupported;
      }
    } catch {}
  }
  lsfgAdaptiveSupported = false;
  return false;
}

function buildLsfgToml(cfg, { forceDisabled = false } = {}) {
  const clampedPct = Math.max(25, Math.min(100, Math.round(cfg.flowScale)));
  const flowFloat = (clampedPct / 100).toFixed(2);
  let multiplier = 2;
  if (forceDisabled || cfg.mode === 'off') multiplier = 1;
  else if (cfg.mode === '3x') multiplier = 3;
  else if (cfg.mode === '4x') multiplier = 4;

  const activeNames = Array.from(new Set([
    'electron',
    'terra',
    'Terra',
    path.basename(process.execPath),
    app.getName()
  ].filter(Boolean)));

  return [
    'version = 2',
    '',
    '[global]',
    `dll = ${JSON.stringify(cfg.dllPath)}`,
    '',
    '[[profile]]',
    'name = "terra"',
    `active_in = [${activeNames.map((n) => JSON.stringify(n)).join(', ')}]`,
    `multiplier = ${multiplier}`,
    `flow_scale = ${flowFloat}`,
    `performance_mode = ${cfg.performanceMode ? 'true' : 'false'}`,
    ''
  ].join('\n');
}

function writeLsfgTomlIfChanged(cfg) {
  if (!isExistingFile(cfg.dllPath)) return;
  const shouldRunNow = isFramegenActiveConfig(cfg) && (!cfg.rideOnly || onRideOrSpectatePage);
  const toml = buildLsfgToml(cfg, { forceDisabled: !shouldRunNow });
  const tomlPath = getLsfgTomlPath();
  if (lastWrittenLsfgToml === null && fs.existsSync(tomlPath)) {
    try { lastWrittenLsfgToml = fs.readFileSync(tomlPath, 'utf8'); } catch {}
  }
  if (toml !== lastWrittenLsfgToml) {
    fs.writeFileSync(tomlPath, toml, 'utf8');
    lastWrittenLsfgToml = toml;
  }
}

function writeFramegenFiles(cfg) {
  const dataDir = getTerraDataDir();
  fs.mkdirSync(dataDir, { recursive: true });
  fs.writeFileSync(getFramegenConfigPath(), JSON.stringify(cfg, null, 2) + '\n', 'utf8');
  writeLsfgTomlIfChanged(cfg);
}

const bootFramegenConfig = readFramegenConfig();
const bootFramegenActive = isFramegenActiveConfig(bootFramegenConfig);
const bootFramegenAdaptive = bootFramegenActive && isAdaptiveMode(bootFramegenConfig.mode) && detectLsfgAdaptiveSupported();
let liveFramegenConfig = bootFramegenActive ? Object.assign({}, bootFramegenConfig) : null;
let earlyRelaunching = false;

function syncFramegenForUrl(rawUrl) {
  const nextOnRide = isRideOrSpectateUrl(rawUrl);
  if (nextOnRide === onRideOrSpectatePage) return;
  onRideOrSpectatePage = nextOnRide;
  if (!bootFramegenActive || bootFramegenAdaptive) return;
  const activeCfg = liveFramegenConfig || bootFramegenConfig;
  if (!activeCfg || !activeCfg.rideOnly || !isExistingFile(activeCfg.dllPath)) return;
  const prevToml = lastWrittenLsfgToml;
  try {
    writeLsfgTomlIfChanged(activeCfg);
    if (lastWrittenLsfgToml !== prevToml) {
      const activeNow = isFramegenActiveConfig(activeCfg) && (!activeCfg.rideOnly || onRideOrSpectatePage);
      console.log(`[electron] Framegen ${activeNow ? `enabled (${activeCfg.mode})` : 'paused (non-ride page)'} for ${rawUrl}`);
    }
  } catch (e) {
    console.warn('[electron] Failed to update lsfg-vk.toml for navigation:', e && e.message ? e.message : e);
  }
}

function getLiveFramegenStats() {
  const cfg = liveFramegenConfig;
  if (!bootFramegenActive || !cfg || !isFramegenActiveConfig(cfg)) {
    return { active: false };
  }
  if (!bootFramegenAdaptive && cfg.rideOnly && !onRideOrSpectatePage) {
    return { active: false, armed: true, rideOnly: true };
  }
  const mode = cfg.mode;
  const adaptiveMode = isAdaptiveMode(mode);
  const adaptiveSupported = adaptiveMode ? detectLsfgAdaptiveSupported() : false;
  let multiplier = 2;
  if (mode === '3x') multiplier = 3;
  else if (mode === '4x') multiplier = 4;
  return {
    active: true,
    mode,
    multiplier,
    adaptive: adaptiveMode && adaptiveSupported,
    targetFps: adaptiveMode ? (mode === 'adaptive120' ? 120 : 60) : null,
    maxMultiplier: 4,
    flowScale: cfg.flowScale
  };
}

function buildRelaunchArgs(enableX11Vulkan) {
  const x11Flags = ['--ozone-platform=x11', '--use-gl=angle', '--use-angle=vulkan'];
  const filtered = process.argv.slice(1).filter((arg, idx, arr) => {
    if (x11Flags.includes(arg)) return false;
    if (enableX11Vulkan && (arg.startsWith('--ozone-platform=') || arg.startsWith('--use-gl=') || arg.startsWith('--use-angle='))) return false;
    if (arg === '--ozone-platform' || arg === '--use-gl' || arg === '--use-angle') return false;
    if (idx > 0 && (arr[idx - 1] === '--ozone-platform' || arr[idx - 1] === '--use-gl' || arr[idx - 1] === '--use-angle')) return false;
    return true;
  });
  return enableX11Vulkan ? filtered.concat(x11Flags) : filtered;
}

function hasX11OzoneArg() {
  if (process.argv.includes('--ozone-platform=x11')) return true;
  const idx = process.argv.indexOf('--ozone-platform');
  return idx !== -1 && process.argv[idx + 1] === 'x11';
}

function applyFramegenEnv(cfg) {
  const active = isFramegenActiveConfig(cfg);
  const adaptiveEnv = active && isAdaptiveMode(cfg.mode) && detectLsfgAdaptiveSupported();
  if (active) {
    delete process.env.DISABLE_LSFGVK;
    process.env.LSFGVK_CONFIG = getLsfgTomlPath();
    if (adaptiveEnv) {
      const clampedPct = Math.max(25, Math.min(100, Math.round(cfg.flowScale)));
      const flowFloat = (clampedPct / 100).toFixed(2);
      delete process.env.LSFGVK_PROFILE;
      process.env.LSFGVK_ENV = '1';
      process.env.LSFGVK_DLL_PATH = cfg.dllPath;
      process.env.LSFGVK_ADAPTIVE = '1';
      process.env.LSFGVK_TARGET_FPS = cfg.mode === 'adaptive120' ? '120' : '60';
      process.env.LSFGVK_ADAPTIVE_MAX_MULTIPLIER = '4';
      process.env.LSFGVK_MULTIPLIER = '2';
      process.env.LSFGVK_FLOW_SCALE = flowFloat;
      process.env.LSFGVK_PERFORMANCE_MODE = cfg.performanceMode ? '1' : '0';
    } else {
      delete process.env.LSFGVK_ENV;
      delete process.env.LSFGVK_DLL_PATH;
      delete process.env.LSFGVK_ADAPTIVE;
      delete process.env.LSFGVK_TARGET_FPS;
      delete process.env.LSFGVK_ADAPTIVE_MAX_MULTIPLIER;
      delete process.env.LSFGVK_MULTIPLIER;
      delete process.env.LSFGVK_FLOW_SCALE;
      delete process.env.LSFGVK_PERFORMANCE_MODE;
      process.env.LSFGVK_PROFILE = 'terra';
    }
    process.env.TERRA_LSFG_READY = '1';
  } else {
    process.env.DISABLE_LSFGVK = '1';
    delete process.env.LSFGVK_CONFIG;
    delete process.env.LSFGVK_PROFILE;
    delete process.env.LSFGVK_ENV;
    delete process.env.LSFGVK_DLL_PATH;
    delete process.env.LSFGVK_ADAPTIVE;
    delete process.env.LSFGVK_TARGET_FPS;
    delete process.env.LSFGVK_ADAPTIVE_MAX_MULTIPLIER;
    delete process.env.LSFGVK_MULTIPLIER;
    delete process.env.LSFGVK_FLOW_SCALE;
    delete process.env.LSFGVK_PERFORMANCE_MODE;
    delete process.env.TERRA_LSFG_READY;
  }
}

function getRelaunchTargetAndArgs(enableX11Vulkan) {
  const execTarget = process.env.APPIMAGE || process.execPath;
  const args = process.env.APPIMAGE
    ? buildRelaunchArgs(enableX11Vulkan).filter((a) => !a.startsWith('--appimage'))
    : buildRelaunchArgs(enableX11Vulkan);
  return { execTarget, args };
}

function relaunchWithEnv(enableX11Vulkan) {
  const { execTarget, args } = getRelaunchTargetAndArgs(enableX11Vulkan);
  const child = spawn(execTarget, args, {
    env: process.env,
    detached: true,
    stdio: 'ignore'
  });
  child.unref();
  app.exit(0);
}

function relaunchForeground(enableX11Vulkan) {
  const { execTarget, args } = getRelaunchTargetAndArgs(enableX11Vulkan);
  const child = spawn(execTarget, args, {
    env: process.env,
    detached: false,
    stdio: 'inherit'
  });
  const forwardSig = (sig) => {
    try {
      if (!child.killed) child.kill(sig);
    } catch {}
  };
  process.on('SIGINT', () => forwardSig('SIGINT'));
  process.on('SIGTERM', () => forwardSig('SIGTERM'));
  child.on('exit', (code, signal) => {
    app.exit(code != null ? code : (signal ? 1 : 0));
  });
  child.on('error', (err) => {
    console.error('[electron] Foreground relaunch failed:', err);
    app.exit(1);
  });
}

if (process.platform === 'linux') {
  if (bootFramegenActive) {
    try {
      writeFramegenFiles(bootFramegenConfig);
    } catch (e) {
      console.warn('[electron] Failed to write lsfg-vk config at boot:', e && e.message ? e.message : e);
    }
    const needRelaunch = !hasX11OzoneArg() ||
      (!process.env.TERRA_LSFG_READY && !process.env.LSFGVK_CONFIG);
    applyFramegenEnv(bootFramegenConfig);
    if (needRelaunch && !process.env.TERRA_X11_RELAUNCHED) {
      earlyRelaunching = true;
      process.env.TERRA_X11_RELAUNCHED = '1';
      console.log('[electron] Relaunching under XWayland + Vulkan with lsfg-vk env...');
      relaunchForeground(true);
    }
  } else {
    applyFramegenEnv(bootFramegenConfig);
  }
}

function computeFramegenRestartRequired(curCfg) {
  const nextActive = isFramegenActiveConfig(curCfg);
  if (nextActive !== bootFramegenActive) return true;
  if (!bootFramegenActive) return false;
  const nextAdaptive = isAdaptiveMode(curCfg.mode) && detectLsfgAdaptiveSupported();
  if (nextAdaptive !== bootFramegenAdaptive) return true;
  if (curCfg.dllPath !== bootFramegenConfig.dllPath) return true;
  if (bootFramegenAdaptive) {
    return (
      curCfg.mode !== bootFramegenConfig.mode ||
      curCfg.flowScale !== bootFramegenConfig.flowScale ||
      curCfg.performanceMode !== bootFramegenConfig.performanceMode
    );
  }
  return false;
}

function getFramegenStatus(overrideCfg) {
  const config = overrideCfg || readFramegenConfig();
  const live = getLiveFramegenStats();
  return {
    supported: process.platform === 'linux',
    layerInstalled: detectLsfgLayerInstalled(),
    dllExists: isExistingFile(config.dllPath),
    bootActive: bootFramegenActive,
    activeNow: Boolean(live.active),
    onRidePage: onRideOrSpectatePage,
    restartRequired: computeFramegenRestartRequired(config),
    configPath: getFramegenConfigPath(),
    tomlPath: getLsfgTomlPath(),
    config
  };
}

// Enable Chromium GPU hardware acceleration and GPU rasterization for WebGL
app.commandLine.appendSwitch('ignore-gpu-blocklist');
app.commandLine.appendSwitch('enable-gpu-rasterization');
app.commandLine.appendSwitch('enable-zero-copy');

// Force the Vulkan graphics pipeline on Linux (AMD RDNA-3: without these the
// default ANGLE path renders with artifacts). Equivalent to enabling the
// chrome://flags entries #enable-vulkan, #default-angle-vulkan and
// #vulkan-from-angle at app startup.
// When Frame Generation (lsfg-vk) is enabled, also switch to XWayland + ANGLE
// Vulkan so Chromium presents via a Vulkan swapchain that lsfg-vk can hook.
if (process.platform === 'linux') {
  app.commandLine.appendSwitch(
    'enable-features',
    'Vulkan,DefaultANGLEVulkan,VulkanFromANGLE'
  );
  if (bootFramegenActive) {
    app.commandLine.appendSwitch('ozone-platform', 'x11');
    app.commandLine.appendSwitch('use-gl', 'angle');
    app.commandLine.appendSwitch('use-angle', 'vulkan');
    console.log('[electron] Frame generation enabled at boot:', bootFramegenConfig.mode, 'dll:', bootFramegenConfig.dllPath);
  }
  if (typeof app.setDesktopName === 'function') {
    app.setDesktopName('terra-desktop.desktop');
  }
}

let mainWindow;
let rustProcess;
let isQuitting = false;
let shutdownPromise = null;
let reqId = 1;
let powerBlockerId = null;
const pendingRpc = new Map();

function startRustProcess() {
  const binaryName = process.platform === 'win32' ? 'terra.exe' : 'terra';
  const candidates = [
    path.join(process.resourcesPath, 'bin', binaryName),
    path.join(__dirname, 'bin', binaryName),
    path.join(__dirname, '..', 'target', 'release', binaryName),
    path.join(__dirname, '..', 'target', 'debug', binaryName)
  ];

  const binaryPath = candidates.find((p) => fs.existsSync(p));

  if (!binaryPath) {
    console.error('[electron] Terra Rust binary not found in candidates:', candidates);
    return;
  }

  // Ensure binary is executable (+x) on Linux and macOS if not already set
  if (process.platform !== 'win32') {
    try {
      const stats = fs.statSync(binaryPath);
      if ((stats.mode & 0o111) === 0) {
        fs.chmodSync(binaryPath, 0o755);
      }
    } catch (_) {}
  }

  console.log('[electron] Spawning real Rust BLE & Addons backend:', binaryPath);
  rustProcess = spawn(binaryPath, ['--rpc'], {
    detached: process.platform !== 'win32',
    windowsHide: true
  });

  let lineBuffer = '';
  rustProcess.stdout.on('data', (data) => {
    lineBuffer += data.toString();
    const lines = lineBuffer.split('\n');
    lineBuffer = lines.pop(); // keep partial line in buffer

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      try {
        const msg = JSON.parse(trimmed);
        if (msg.id !== undefined && pendingRpc.has(msg.id)) {
          const { resolve, reject } = pendingRpc.get(msg.id);
          pendingRpc.delete(msg.id);
          if (msg.error) reject(new Error(msg.error));
          else resolve(msg.result);
        } else if (msg.event === 'js_eval' && msg.code && mainWindow) {
          mainWindow.webContents.executeJavaScript(msg.code).catch(() => {});
        } else if (msg.event && mainWindow) {
          mainWindow.webContents.send('rust-event', msg);
        } else {
          console.log('[rust]', trimmed);
        }
      } catch {
        console.log('[rust]', trimmed);
      }
    }
  });

  rustProcess.stderr.on('data', (data) => {
    console.error('[rust-stderr]', data.toString().trim());
  });

  rustProcess.on('exit', (code, signal) => {
    console.error(`[electron] Rust process exited with code ${code}, signal ${signal}.`);
    rustProcess = null;
    for (const [id, entry] of pendingRpc) {
      pendingRpc.delete(id);
      entry.reject(new Error(`Rust backend exited (code ${code}, signal ${signal})`));
    }
    if (!isQuitting) {
      setTimeout(() => {
        if (!isQuitting && mainWindow && !mainWindow.isDestroyed()) {
          startRustProcess();
        }
      }, 1000);
    }
  });

  rustProcess.on('error', (err) => {
    console.error('[electron] Rust process error:', err);
  });
}

// Send one command to the Rust backend over stdio. Resolves with the result,
// or with { __error: message } if the backend reports an error.
function sendRpc(cmd, args) {
  if (!rustProcess || !rustProcess.stdin.writable) {
    return Promise.resolve({ __error: 'Rust backend process not running' });
  }
  const id = reqId++;
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      if (pendingRpc.has(id)) {
        pendingRpc.delete(id);
        resolve({ __error: `RPC timeout for command: ${cmd}` });
      }
    }, 30000);

    pendingRpc.set(id, {
      resolve: (val) => {
        clearTimeout(timer);
        if (cmd === 'addons_activate' && val && Array.isArray(val.scripts) && mainWindow) {
          for (const s of val.scripts) {
            if (s.code) {
              mainWindow.webContents.executeJavaScript(s.code).catch((e) => console.error('[addon-exec-error]', e));
            }
          }
        }
        resolve(val);
      },
      reject: (err) => {
        clearTimeout(timer);
        resolve({ __error: err.message || String(err) });
      }
    });

    rustProcess.stdin.write(JSON.stringify({ id, cmd, args: args || {} }) + '\n');
  });
}

let prevNodeCpus = null;

function readNodeSystemCpuPct() {
  try {
    const cur = os.cpus();
    if (!cur || !cur.length) return null;
    if (!prevNodeCpus || prevNodeCpus.length !== cur.length) {
      prevNodeCpus = cur;
      return null;
    }
    let idleDelta = 0;
    let totalDelta = 0;
    for (let i = 0; i < cur.length; i++) {
      const a = prevNodeCpus[i].times;
      const b = cur[i].times;
      const dIdle = Math.max(0, b.idle - a.idle);
      const dTotal = Math.max(0, (b.user - a.user) + (b.nice - a.nice) + (b.sys - a.sys) + (b.irq - a.irq) + dIdle);
      idleDelta += dIdle;
      totalDelta += dTotal;
    }
    prevNodeCpus = cur;
    if (totalDelta <= 0) return null;
    const pct = ((totalDelta - idleDelta) / totalDelta) * 100;
    return Math.round(Math.max(0, Math.min(100, pct)) * 10) / 10;
  } catch {
    return null;
  }
}

function mergeElectronPerfStats(rustStats) {
  const out = Object.assign({ platform: process.platform }, rustStats || {});
  const numCpus = Math.max(1, (os.cpus() && os.cpus().length) || 1);

  try {
    const metrics = app.getAppMetrics();
    let totalCpu = 0;
    let gpuCpu = 0;
    let rendererCpu = 0;
    let totalMemKb = 0;
    for (const m of metrics) {
      const c = (m.cpu && typeof m.cpu.percentCPUUsage === 'number') ? m.cpu.percentCPUUsage : 0;
      totalCpu += c;
      if (m.type === 'GPU') gpuCpu += c;
      else if (m.type === 'Tab') rendererCpu += c;
      if (m.memory && typeof m.memory.workingSetSize === 'number') {
        totalMemKb += m.memory.workingSetSize;
      }
    }
    const round1 = (v) => Math.round(v * 10) / 10;
    if (out.appCpuPct == null) out.appCpuPct = round1(Math.min(100, totalCpu / numCpus));
    out.gpuProcCpuPct = round1(Math.min(100, gpuCpu / numCpus));
    out.rendererCpuPct = round1(Math.min(100, rendererCpu / numCpus));
    out.appMemMb = round1(totalMemKb / 1024);
  } catch {}

  const nodeCpuPct = readNodeSystemCpuPct();
  if (out.cpuPct == null && nodeCpuPct != null) {
    out.cpuPct = nodeCpuPct;
  }

  if (out.memTotalMb == null) {
    try {
      const totalMb = os.totalmem() / (1024 * 1024);
      const usedMb = Math.max(0, os.totalmem() - os.freemem()) / (1024 * 1024);
      out.memTotalMb = Math.round(totalMb * 10) / 10;
      out.memUsedMb = Math.round(usedMb * 10) / 10;
    } catch {}
  }

  out.framegen = getLiveFramegenStats();

  return out;
}

ipcMain.handle('tauri-invoke', async (event, payload) => {
  const { cmd, args } = payload || {};
  if (!cmd) return null;
  if (cmd === 'perf_stats') {
    const rustStats = await sendRpc(cmd, args).catch(() => null);
    return mergeElectronPerfStats(rustStats && !rustStats.__error ? rustStats : {});
  }
  if (cmd === 'framegen_get') {
    return getFramegenStatus();
  }
  if (cmd === 'framegen_set') {
    const prev = readFramegenConfig();
    const next = normalizeFramegenConfig({
      mode: args && args.mode !== undefined ? args.mode : prev.mode,
      dllPath: args && args.dllPath !== undefined ? args.dllPath : prev.dllPath,
      flowScale: args && args.flowScale !== undefined ? args.flowScale : prev.flowScale,
      performanceMode: args && args.performanceMode !== undefined ? args.performanceMode : prev.performanceMode,
      rideOnly: args && args.rideOnly !== undefined ? args.rideOnly : prev.rideOnly
    });
    try {
      writeFramegenFiles(next);
    } catch (e) {
      return { __error: e && e.message ? e.message : String(e) };
    }
    const nextAdaptiveEnv = isAdaptiveMode(next.mode) && detectLsfgAdaptiveSupported();
    if (
      bootFramegenActive &&
      !bootFramegenAdaptive &&
      !nextAdaptiveEnv &&
      next.dllPath === bootFramegenConfig.dllPath
    ) {
      liveFramegenConfig = Object.assign({}, next);
    }
    return getFramegenStatus(next);
  }
  if (cmd === 'framegen_pick_dll') {
    if (process.platform !== 'linux') {
      return { __error: 'Frame generation is only supported on Linux' };
    }
    const cur = readFramegenConfig();
    let defaultPath = cur.dllPath || detectDefaultDllPath();
    if (defaultPath && fs.existsSync(defaultPath)) {
      defaultPath = path.dirname(defaultPath);
    } else {
      defaultPath = os.homedir();
    }
    const res = await dialog.showOpenDialog(mainWindow || undefined, {
      title: 'Select Lossless Scaling DLL (Lossless.dll or lsfg-vk.dll)',
      defaultPath,
      properties: ['openFile'],
      filters: [
        { name: 'Lossless Scaling DLL', extensions: ['dll'] },
        { name: 'All Files', extensions: ['*'] }
      ]
    });
    if (res.canceled || !res.filePaths || !res.filePaths.length) {
      return Object.assign({ cancelled: true }, getFramegenStatus(cur));
    }
    const next = normalizeFramegenConfig(Object.assign({}, cur, { dllPath: res.filePaths[0] }));
    try {
      writeFramegenFiles(next);
    } catch (e) {
      return { __error: e && e.message ? e.message : String(e) };
    }
    return Object.assign({ cancelled: false }, getFramegenStatus(next));
  }
  if (cmd === 'framegen_relaunch') {
    await gracefulShutdownRust({ dispatchPagehide: true });
    const nextCfg = readFramegenConfig();
    const nextActive = isFramegenActiveConfig(nextCfg);
    delete process.env.TERRA_X11_RELAUNCHED;
    applyFramegenEnv(nextCfg);
    relaunchWithEnv(nextActive);
    return { relaunching: true };
  }
  return sendRpc(cmd, args);
});

// ---------------------------------------------------------------------------
// Auto-breakpoint for Biketerra internals.
//
// The game keeps its manager (the Tf class) inside a module closure; addons
// need it on `window`.  Terra reproduces the old manual "conditional
// breakpoint" trick via the CDP debugger: find the line
// `this.riderController.handleInput()` inside the (per-deploy hashed) game
// chunk, and set a breakpoint whose condition is `window.btApp = this, false`.
// The condition publishes `this` and evaluates to `false`, so the game
// *never pauses*.
//
// Resolves the chunk dynamically on every full page load, so it survives
// Biketerra's redeploys (new chunk hash, relocated line).
//
// Disable with: TERRA_NO_BT_BREAKPOINT=1
// Note: while Terra's debugger client is attached, the page's own DevTools
// cannot attach (Chromium allows one debugger client per target).
// ---------------------------------------------------------------------------
const BT_APP_MARKER = /this\.riderController\.handleInput\(\s*\)/;
const btapp = { attached: false, bpId: null, busy: false };

// Returns { url, lineNumber, columnNumber } of the loaded game chunk containing
// the marker, or null. Both coords are 0-based (CDP convention). The game chunk
// is minified into very long single lines, so `columnNumber` is essential: a
// line-only breakpoint lands on the line's FIRST statement (a setup declaration
// that runs once), never on the per-call marker deep in the line.
async function btappFindChunk(wc) {
  let urls = [];
  try {
    urls = await wc.executeJavaScript(
      `Array.from(performance.getEntriesByType('resource'), (e) => e.name)` +
      `.filter((u) => u.includes('/_app/immutable/chunks/'))`,
      false
    );
  } catch {
    return null;
  }
  for (const u of urls || []) {
    try {
      const res = await net.fetch(u, { cache: 'no-cache' });
      if (!res.ok) continue;
      const src = await res.text();
      const m = src.match(BT_APP_MARKER);
      if (!m) continue;
      const lineNumber = src.slice(0, m.index).split('\n').length - 1;
      const lineStart = src.lastIndexOf('\n', m.index - 1) + 1;
      const columnNumber = m.index - lineStart;
      return { url: u, lineNumber, columnNumber };
    } catch {
      /* chunk unreadable — try the next */
    }
  }
  return null;
}

// Wait for the condition to fire (next game tick) and confirm `window.btApp`.
async function btappVerify(wc) {
  for (let i = 0; i < 40; i++) {
    if (isQuitting || wc.isDestroyed()) return;
    await new Promise((r) => setTimeout(r, 500));
    if (isQuitting || wc.isDestroyed()) return;
    let ok = false;
    try {
      ok = await wc.executeJavaScript('!!window.btApp', false);
    } catch {
      /* navigation in flight — keep trying */
    }
    if (ok) {
      console.log('[electron] auto-breakpoint OK: game published window.btApp itself');
      return;
    }
  }
  console.warn('[electron] auto-breakpoint: set, but window.btApp still not seen (game not ticking?)');
}

async function btappEnsure(wc) {
  if (process.env.TERRA_NO_BT_BREAKPOINT === '1') return;
  if (btapp.busy) return;
  btapp.busy = true;
  try {
    if (!btapp.attached) {
      try {
        await wc.debugger.attach('1.3');
      } catch (e) {
        console.warn('[electron] auto-breakpoint: attach failed (' + (e && e.message) + ') — skipped this load');
        return;
      }
      btapp.attached = true;
      await wc.debugger.sendCommand('Debugger.enable');
    }

    // The game chunk may load a few frames after finish-load; retry briefly.
    let hit = null;
    for (let i = 0; i < 15 && !hit; i++) {
      hit = await btappFindChunk(wc);
      if (!hit) await new Promise((r) => setTimeout(r, 400));
    }
    if (!hit) {
      console.warn('[electron] auto-breakpoint: marker line not found in loaded chunks yet');
      return;
    }

    try {
      if (btapp.bpId) {
        await wc.debugger.sendCommand('Debugger.removeBreakpoint', { breakpointId: btapp.bpId });
      }
    } catch {
      /* old id already gone */
    }

    // NOTE: setBreakpointByUrl — NOT setBreakpoint (that one requires a scriptId).
    // columnNumber is REQUIRED for minified single-line chunks: without it the
    // breakpoint snaps to the line's first statement and never fires (verified).
    const set = await wc.debugger.sendCommand('Debugger.setBreakpointByUrl', {
      url: hit.url,
      lineNumber: hit.lineNumber,
      columnNumber: hit.columnNumber,
      condition: 'window.btApp = this, false',
    });
    if (set && set.breakpointId) btapp.bpId = set.breakpointId;
    console.log('[electron] auto-breakpoint set at ' + hit.url.split('/').pop() +
      ':' + (hit.lineNumber + 1) + ':' + hit.columnNumber);
    btappVerify(wc);
  } catch (e) {
    console.warn('[electron] auto-breakpoint error:', e && e.message ? e.message : e);
  } finally {
    btapp.busy = false;
  }
}

function resolveWindowIconPath() {
  const iconFile = process.platform === 'win32' ? 'icon.ico' : 'icon.png';
  const candidates = [
    path.join(process.resourcesPath, 'icons', iconFile),
    path.join(__dirname, '..', 'icons', iconFile)
  ];
  return candidates.find((p) => {
    try { return fs.existsSync(p); } catch { return false; }
  }) || null;
}

function ensureLinuxAppImageIcon(iconSrc) {
  if (process.platform !== 'linux' || !process.env.APPIMAGE || !iconSrc) return;
  try {
    const xdgData = process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share');
    const iconDir = path.join(xdgData, 'icons', 'hicolor', '512x512', 'apps');
    const iconDest = path.join(iconDir, 'terra-desktop.png');
    fs.mkdirSync(iconDir, { recursive: true });

    const srcBuf = fs.readFileSync(iconSrc);
    let needIconWrite = true;
    try {
      if (fs.existsSync(iconDest) && fs.readFileSync(iconDest).equals(srcBuf)) {
        needIconWrite = false;
      }
    } catch {}
    if (needIconWrite) {
      fs.writeFileSync(iconDest, srcBuf);
    }

    const appsDir = path.join(xdgData, 'applications');
    const desktopPath = path.join(appsDir, 'terra-desktop.desktop');
    const managedMarker = '# Managed by Terra AppImage';
    if (fs.existsSync(desktopPath)) {
      const existing = fs.readFileSync(desktopPath, 'utf8');
      if (!existing.includes(managedMarker)) {
        // Respect user- or Gear Lever/AppImageLauncher-managed desktop entries.
        return;
      }
    }

    fs.mkdirSync(appsDir, { recursive: true });
    const escapedExec = process.env.APPIMAGE.replace(/([\\"$`])/g, '\\$1');
    const desktopContent = [
      '[Desktop Entry]',
      managedMarker,
      'Type=Application',
      'Name=Terra',
      'Comment=Biketerra Companion App',
      `Icon=${iconDest}`,
      `Exec="${escapedExec}" %U`,
      'Terminal=false',
      'Categories=Sports;',
      'StartupWMClass=terra-desktop',
      'NoDisplay=true',
      ''
    ].join('\n');

    let needDesktopWrite = true;
    try {
      if (fs.existsSync(desktopPath) && fs.readFileSync(desktopPath, 'utf8') === desktopContent) {
        needDesktopWrite = false;
      }
    } catch {}
    if (needDesktopWrite) {
      fs.writeFileSync(desktopPath, desktopContent, 'utf8');
    }
  } catch (e) {
    console.warn('[electron] Failed to install Wayland AppImage icon shim:', e && e.message ? e.message : e);
  }
}

function createWindow() {
  Menu.setApplicationMenu(null);

  const windowIcon = resolveWindowIconPath();
  ensureLinuxAppImageIcon(windowIcon);

  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    title: 'Terra',
    ...(windowIcon ? { icon: windowIcon } : {}),
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(__dirname, 'preload.js'),
      contextIsolation: false,
      nodeIntegration: false,
      sandbox: false,
      webSecurity: true,
      experimentalFeatures: true
    }
  });

  mainWindow.setMenu(null);

  // This app has no application menu, so wire the DevTools shortcut directly.
  mainWindow.webContents.on('before-input-event', (event, input) => {
    if (input.type !== 'keyDown') return;
    const isToggle = (input.control && input.shift &&
      (input.code === 'KeyI' || (input.key || '').toLowerCase() === 'i')) || input.code === 'F12';
    if (isToggle) {
      event.preventDefault();
      mainWindow.webContents.toggleDevTools({ mode: 'detach' });
    }
  });

  mainWindow.loadURL('https://biketerra.com');

  // Auto-breakpoint: expose window.btApp without manual DevTools (see above).
  mainWindow.webContents.debugger.on('detach', () => {
    btapp.attached = false;
    btapp.bpId = null;
    console.log('[electron] debugger client detached (DevTools opened?). Auto-breakpoint re-arms on next navigation.');
  });
  const arm = () => {
    if (!mainWindow.isDestroyed() && mainWindow.webContents.getURL().includes('biketerra.com')) {
      btappEnsure(mainWindow.webContents).catch((e) => console.warn('[electron] auto-breakpoint:', e));
    }
  };
  const onNav = () => {
    if (!mainWindow || mainWindow.isDestroyed()) return;
    syncFramegenForUrl(mainWindow.webContents.getURL());
    arm();
  };
  // Full loads AND SPA navigations: the game chunk is lazy, and only loads
  // once a user enters a ride. An in-flight scan loop re-reads the live chunk
  // list, so it picks the chunk up whenever it appears.
  mainWindow.webContents.on('did-navigate', onNav);
  mainWindow.webContents.on('did-finish-load', onNav);
  mainWindow.webContents.on('did-navigate-in-page', onNav);

  mainWindow.on('close', (event) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      event.preventDefault();
      const win = mainWindow;
      try { win.hide(); } catch {}
      gracefulShutdownRust({ dispatchPagehide: true }).finally(() => {
        if (win && !win.isDestroyed()) {
          win.destroy();
        }
      });
    }
  });

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

function gracefulShutdownRust({ dispatchPagehide = false } = {}) {
  isQuitting = true;
  if (powerBlockerId !== null && powerSaveBlocker.isStarted(powerBlockerId)) {
    powerSaveBlocker.stop(powerBlockerId);
  }
  if (shutdownPromise) return shutdownPromise;
  shutdownPromise = (async () => {
    try {
      if (btapp.attached && mainWindow && !mainWindow.isDestroyed()) {
        mainWindow.webContents.debugger.detach();
      }
    } catch {}
    if (dispatchPagehide && mainWindow && !mainWindow.isDestroyed()) {
      try {
        await Promise.race([
          mainWindow.webContents.executeJavaScript("window.dispatchEvent(new Event('pagehide'))", false),
          new Promise((r) => setTimeout(r, 300))
        ]);
      } catch {}
    }
    if (rustProcess) {
      // Give in-flight pagehide addon teardown (e.g. Kickr WFTNP 0x2AD9 [01]
      // control release + addon_tcp_close's 250ms writer drain & FIN) time to
      // flush before killing the Rust backend, while disconnecting BLE devices.
      const tcpDrainGrace = new Promise((r) => setTimeout(r, 450));
      const bleDisconnect = (async () => {
        try {
          const r = await Promise.race([
            sendRpc('ble_disconnect_all', {}),
            new Promise((r) => setTimeout(() => r({ __timeout: true }), 8000))
          ]);
          if (r && r.__timeout) console.warn('[electron] ble_disconnect_all timed out after 8000ms');
          else if (r && r.__error) console.error('[electron] ble_disconnect_all failed:', r.__error);
        } catch (e) {
          console.error('[electron] ble_disconnect_all failed:', e);
        }
      })();
      await Promise.all([bleDisconnect, tcpDrainGrace]);
      if (rustProcess) {
        try { rustProcess.stdin.end(); } catch {}
        try { rustProcess.kill(); } catch {}
        rustProcess = null;
      }
    }
  })();
  return shutdownPromise;
}

app.whenReady().then(() => {
  if (earlyRelaunching) return;
  // Prevent display sleep, dimming, and screensaver while Terra is running
  powerBlockerId = powerSaveBlocker.start('prevent-display-sleep');
  console.log('[electron] Activated power save blocker (id:', powerBlockerId, ')');

  // Prime CPU delta baselines for perf_stats
  try { app.getAppMetrics(); } catch {}
  readNodeSystemCpuPct();

  startRustProcess();
  createWindow();

  const handleTermSignal = () => {
    gracefulShutdownRust({ dispatchPagehide: true }).finally(() => {
      app.exit(0);
    });
  };
  process.on('SIGINT', handleTermSignal);
  process.on('SIGTERM', handleTermSignal);

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on('window-all-closed', async () => {
  if (earlyRelaunching) return;
  await gracefulShutdownRust({ dispatchPagehide: false });
  if (process.platform !== 'darwin') {
    app.quit();
    setTimeout(() => app.exit(0), 1500).unref();
  }
});
