const { app, BrowserWindow, ipcMain, Menu, powerSaveBlocker } = require('electron');
const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');

// Enable Chromium GPU hardware acceleration and GPU rasterization for WebGL
app.commandLine.appendSwitch('ignore-gpu-blocklist');
app.commandLine.appendSwitch('enable-gpu-rasterization');
app.commandLine.appendSwitch('enable-zero-copy');

// Force the Vulkan graphics pipeline on Linux (AMD RDNA-3: without these the
// default ANGLE path renders with artifacts). Equivalent to enabling the
// chrome://flags entries #enable-vulkan, #default-angle-vulkan and
// #vulkan-from-angle at app startup.
if (process.platform === 'linux') {
  app.commandLine.appendSwitch(
    'enable-features',
    'Vulkan,DefaultANGLEVulkan,VulkanFromANGLE'
  );
}

let mainWindow;
let rustProcess;
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
  rustProcess = spawn(binaryPath, ['--rpc']);

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
    console.error(`[electron] Rust process exited with code ${code}, signal ${signal}. Restarting...`);
    rustProcess = null;
    setTimeout(() => {
      if (mainWindow && !mainWindow.isDestroyed()) {
        startRustProcess();
      }
    }, 1000);
  });

  rustProcess.on('error', (err) => {
    console.error('[electron] Rust process error:', err);
  });
}

// Send one command to the Rust backend over stdio. Resolves with the result,
// or with { __error: message } if the backend reports an error.
function sendRpc(cmd, args) {
  if (!rustProcess || !rustProcess.stdin.writable) {
    return Promise.reject(new Error('Rust backend process not running'));
  }
  const id = reqId++;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      if (pendingRpc.has(id)) {
        pendingRpc.delete(id);
        reject(new Error(`RPC timeout for command: ${cmd}`));
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

ipcMain.handle('tauri-invoke', (event, payload) => {
  const { cmd, args } = payload || {};
  if (!cmd) return null;
  return sendRpc(cmd, args);
});

function createWindow() {
  Menu.setApplicationMenu(null);

  mainWindow = new BrowserWindow({
    width: 1400,
    height: 900,
    title: 'Terra',
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

  mainWindow.on('closed', () => {
    mainWindow = null;
  });
}

app.whenReady().then(() => {
  // Prevent display sleep, dimming, and screensaver while Terra is running
  powerBlockerId = powerSaveBlocker.start('prevent-display-sleep');
  console.log('[electron] Activated power save blocker (id:', powerBlockerId, ')');

  startRustProcess();
  createWindow();

  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on('window-all-closed', async () => {
  if (powerBlockerId !== null && powerSaveBlocker.isStarted(powerBlockerId)) {
    powerSaveBlocker.stop(powerBlockerId);
  }
  if (rustProcess) {
    // Tear down all BLE connections inside the Rust process before killing it,
    // so devices don't stay connected to the host. Bounded by the 30s RPC timeout.
    try {
      const r = await sendRpc('ble_disconnect_all', {});
      if (r && r.__error) console.error('[electron] ble_disconnect_all failed:', r.__error);
    } catch (e) {
      console.error('[electron] ble_disconnect_all failed:', e);
    }
    rustProcess.kill();
  }
  if (process.platform !== 'darwin') {
    app.quit();
  }
});
