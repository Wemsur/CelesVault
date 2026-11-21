// 只读仓库版本；不创建 tag、不修改文件版本、不触发构建。
import fs from 'node:fs';
import { execFileSync } from 'node:child_process';

const pkg = JSON.parse(fs.readFileSync('package.json', 'utf8'));
const tauri = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8'));
const cargo = fs.readFileSync('Cargo.toml', 'utf8');
const workspace = cargo.split('[workspace.package]')[1]?.split(/^\[/m)[0];
const cargoVersion = workspace?.match(/^version\s*=\s*"([^"]+)"\s*$/m)?.[1];
const version = pkg.version;
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?$/;
if (typeof version !== 'string' || !semver.test(version)) throw new Error('package.json version 必须是无构建元数据的 SemVer，如 0.2.0 或 0.2.1-beta.1');
if (version !== tauri.version || version !== cargoVersion) throw new Error(`版本不一致：package.json=${version}，tauri.conf.json=${tauri.version}，Cargo.toml=${cargoVersion}`);
for (const file of ['Cargo.lock', 'pnpm-lock.yaml']) {
  execFileSync('git', ['ls-files', '--error-unmatch', '--', file], { stdio: 'pipe' });
}
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error('不能确定完整构建提交');
const tag = `v${version}`;
execFileSync('git', ['check-ref-format', `refs/tags/${tag}`]);
const values = { version, tag, commit, prerelease: String(version.includes('-')) };
if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(values).map(([key, value]) => `${key}=${value}\n`).join(''));
console.log(JSON.stringify(values, null, 2));
