const { downloadArtifact } = require('@electron/get');
const extract = require('extract-zip');
const path = require('path');
const fs = require('fs');

async function main() {
  console.log('[terra-electron] Downloading Electron binary for Linux x64...');
  const zipPath = await downloadArtifact({
    version: '33.4.11',
    artifactName: 'electron',
    platform: 'linux',
    arch: 'x64'
  });
  console.log('[terra-electron] Extracting Electron binary from:', zipPath);
  const distDir = path.resolve(__dirname, 'node_modules/electron/dist');
  await extract(zipPath, { dir: distDir });
  fs.writeFileSync(path.resolve(__dirname, 'node_modules/electron/path.txt'), 'electron');
  console.log('[terra-electron] Successfully installed Electron binary!');
}

main().catch(err => {
  console.error('[terra-electron] Error installing Electron:', err);
  process.exit(1);
});
