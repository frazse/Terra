const { app, BrowserWindow, ipcMain, Menu, net, powerSaveBlocker } = require('electron');
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
    await new Promise((r) => setTimeout(r, 500));
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
  // Full loads AND SPA navigations: the game chunk is lazy, and only loads
  // once a user enters a ride. An in-flight scan loop re-reads the live chunk
  // list, so it picks the chunk up whenever it appears.
  mainWindow.webContents.on('did-finish-load', arm);
  mainWindow.webContents.on('did-navigate-in-page', arm);

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
