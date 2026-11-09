const fs = require('node:fs');
const path = require('node:path');
const cp = require('node:child_process');

const root = path.resolve(__dirname, '..');
const outDir = path.join(root, 'outputs', 'OrangeSchedule-1.2.1-Portable');
const zipPath = path.join(root, 'outputs', 'OrangeSchedule-1.2.1-Portable.zip');
const exeSrc = path.join(root, 'src-tauri', 'target', 'release', 'weeklight.exe');

if (!fs.existsSync(exeSrc)) {
  console.error('Source executable not found:', exeSrc);
  process.exit(1);
}

fs.rmSync(outDir, { recursive: true, force: true });
fs.mkdirSync(outDir, { recursive: true });

// Copy executable
fs.copyFileSync(exeSrc, path.join(outDir, '橙子日程.exe'));

// Create portable marker file
fs.writeFileSync(path.join(outDir, '.portable'), '');

// Create instructions
const readme = [
  '橙子日程 · 免安装绿色便携版 (v1.2.1)',
  '=====================================',
  '',
  '【使用说明】',
  '1. 直接双击运行「橙子日程.exe」即可使用，无需安装，无外部依赖。',
  '2. 目录内包含「.portable」标记文件，所有日程数据、便签位置与个性化设置将保存在本目录下的「data」文件夹中。',
  '3. 您可以将整个文件夹放置在电脑任意位置，或直接拷贝至 U 盘随身携带使用。',
  '4. 如需切换回系统全局目录（%APPDATA%），删除「.portable」文件与「data」文件夹即可。',
  '',
  '【系统要求】',
  'Windows 10 / 11 x64（系统自带 WebView2 运行时）',
  ''
].join('\r\n');

fs.writeFileSync(path.join(outDir, '使用说明.txt'), readme, 'utf8');

// Compress using powershell Compress-Archive
if (fs.existsSync(zipPath)) fs.unlinkSync(zipPath);
cp.execSync(`powershell -NoProfile -Command "Compress-Archive -Path '${outDir}\\*' -DestinationPath '${zipPath}' -CompressionLevel Optimal"`, { stdio: 'inherit' });

const stats = fs.statSync(zipPath);
console.log(`Portable package created successfully!`);
console.log(`ZIP: ${zipPath}`);
console.log(`Size: ${(stats.size / 1024 / 1024).toFixed(2)} MB (${stats.size} bytes)`);
