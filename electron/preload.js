const { ipcRenderer, webFrame } = require('electron');
const fs = require('fs');
const path = require('path');

// Expose Tauri IPC bridge for bluetooth-shim.js and addons-loader.js
window.__TAURI_INTERNALS__ = {
  invoke: async (cmd, args) => {
    const res = await ipcRenderer.invoke('tauri-invoke', { cmd, args });
    if (res && res.__error) {
      throw res.__error;
    }
    return res;
  }
};

// Listen for rust-event messages from main.js (Rust backend stdout events).
// These carry chooser_open, ble_notify, etc. and are routed to window.__terra.
ipcRenderer.on('rust-event', (event, msg) => {
  if (!window.__terra) return;
  try {
    if (msg.event === 'chooser_open') {
      window.__terra.showChooserModal();
    } else if (msg.event === 'ble_notify') {
      if (msg.type === 'disconnected' && msg.device) {
        window.__terra.disconnected(msg.device);
      } else if (msg.type === 'characteristic' && msg.device && msg.characteristic && msg.value) {
        window.__terra.notify(msg.device, msg.characteristic, msg.value);
      }
    }
  } catch (e) {
    console.error('[terra-electron] rust-event handler error:', e);
  }
});

try {
  const rootDir = path.join(__dirname, '..');
  let shimPath = path.join(rootDir, 'shim', 'bluetooth-shim.js');
  let loaderPath = path.join(rootDir, 'shim', 'addons-loader.js');
  let uiPath = path.join(rootDir, 'shim', 'terra-ui.js');

  if (!fs.existsSync(shimPath)) {
    shimPath = path.join(process.resourcesPath, 'shim', 'bluetooth-shim.js');
    loaderPath = path.join(process.resourcesPath, 'shim', 'addons-loader.js');
    uiPath = path.join(process.resourcesPath, 'shim', 'terra-ui.js');
  }

  const shim = fs.readFileSync(shimPath, 'utf8');
  const loader = fs.readFileSync(loaderPath, 'utf8');
  const ui = fs.readFileSync(uiPath, 'utf8');

  // Read Terra's remembered non-addon device IDs synchronously so bluetooth-shim.js
  // can seed Biketerra's native `saved-devices` before Biketerra's JS bundle loads.
  let rememberedIds = [];
  try {
    const os = require('os');
    const dataDir = process.platform === 'linux'
      ? path.join(process.env.XDG_DATA_HOME || path.join(os.homedir(), '.local', 'share'), 'terra')
      : process.platform === 'win32'
        ? path.join(process.env.LOCALAPPDATA || path.join(os.homedir(), 'AppData', 'Local'), 'terra')
        : path.join(os.homedir(), 'Library', 'Application Support', 'terra');
    const raw = fs.readFileSync(path.join(dataDir, 'devices.json'), 'utf8');
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      rememberedIds = parsed.filter((d) => d && d.id && !d.owner).map((d) => d.id);
    }
  } catch {}

  const desktopBoot = `window.__terraDesktop = true; window.__terraRememberedIds = ${JSON.stringify(rememberedIds)};`;
  const fullInit = `${desktopBoot}\n;\n${shim}\n;\n${loader}\n;\n${ui}`;

  // Execute in main world so page JS sees navigator.bluetooth, window.native, and shims
  webFrame.executeJavaScriptInIsolatedWorld(0, [
    { code: fullInit }
  ]);
  console.log('[terra-electron] Shims successfully injected into main world');
} catch (err) {
  console.error('[terra-electron] Error loading shims in preload:', err);
}
