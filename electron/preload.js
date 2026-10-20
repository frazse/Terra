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

  const fullInit = `${shim}\n;\n${loader}\n;\n${ui}`;

  // Execute in main world so page JS sees navigator.bluetooth and shims
  webFrame.executeJavaScriptInIsolatedWorld(0, [
    { code: fullInit }
  ]);
  console.log('[terra-electron] Shims successfully injected into main world');
} catch (err) {
  console.error('[terra-electron] Error loading shims in preload:', err);
}
