// 仅由手动工作流的发布 job 调用；不会修改已有 tag 或 Release。
import fs from 'node:fs';
import path from 'node:path';
import { createHash } from 'node:crypto';

const { GH_TOKEN: token, GITHUB_REPOSITORY: repo, RELEASE_VERSION: version, RELEASE_COMMIT: commit } = process.env;
if (!token || !/^[\w.-]+\/[\w.-]+$/.test(repo ?? '') || !/^[0-9a-f]{40}$/.test(commit ?? '') || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version ?? '')) throw new Error('缺少有效发布参数');
const tag = `v${version}`;
const base = `${process.env.GITHUB_API_URL || 'https://api.github.com'}/repos/${repo}`;
async function api(url, options = {}, optional = false) {
  const response = await fetch(url, { ...options, headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28', ...options.headers } });
  if (optional && response.status === 404) return null;
  if (!response.ok) throw new Error(`GitHub ${response.status}: ${(await response.text()).slice(0, 1000)}`);
  return response.json();
}
const directory = 'release-assets';
const info = JSON.parse(fs.readFileSync(path.join(directory, 'build-info.json'), 'utf8'));
if (info.version !== version || info.commit !== commit || info.tag !== tag) throw new Error('构建产物的版本/提交与发布 job 不一致');
const expected = [`CelesVault-${version}-windows-x64.exe`, `CelesVault-${version}-windows-x64-setup.exe`, `CelesVault-${version}-source.zip`, 'LICENSE', 'THIRD_PARTY_NOTICES.md', 'RELEASE-NOTES.md', 'build-info.json'];
const entries = fs.readFileSync(path.join(directory, 'SHA256SUMS.txt'), 'utf8').trim().split(/\r?\n/);
const checksums = new Map();
for (const line of entries) {
  const match = /^([0-9a-f]{64})  ([^/\\]+)$/.exec(line);
  if (!match || checksums.has(match[2])) throw new Error('校验清单格式无效或有重复项');
  checksums.set(match[2], match[1]);
}
if (checksums.size !== expected.length) throw new Error('交付文件校验数量不符');
for (const name of expected) {
  const file = path.join(directory, name);
  if (!fs.lstatSync(file).isFile() || createHash('sha256').update(fs.readFileSync(file)).digest('hex') !== checksums.get(name)) throw new Error(`产物校验失败：${name}`);
}
// 草稿也必须检查；不覆盖/续传未知构建，不擅自改动已公开版本。
for (let page = 1; ; page++) {
  if (page > 100) throw new Error('Release 数量过多，未安全完成重复版本检查');
  const releases = await api(`${base}/releases?per_page=100&page=${page}`);
  if (releases.some(release => release.tag_name === tag)) throw new Error(`${tag} 已有草稿或正式 Release；请处理旧草稿或提升仓库版本后再运行，不会覆盖`);
  if (releases.length < 100) break;
}
const ref = await api(`${base}/git/ref/tags/${encodeURIComponent(tag)}`, {}, true);
if (ref) {
  let object = ref.object;
  for (let depth = 0; object.type === 'tag' && depth < 10; depth++) object = (await api(`${base}/git/tags/${object.sha}`)).object;
  if (object.type !== 'commit' || object.sha !== commit) throw new Error(`${tag} 已指向其他提交；不会移动或覆盖 tag`);
}
const release = await api(`${base}/releases`, {
  method: 'POST', headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({ tag_name: tag, target_commitish: commit, name: `CelesVault ${version}`, body: fs.readFileSync(path.join(directory, 'RELEASE-NOTES.md'), 'utf8'), draft: true, prerelease: version.includes('-'), make_latest: 'false' }),
});
if (!release.draft) throw new Error('GitHub 返回非草稿状态，已停止上传');
try {
  for (const name of [...expected, 'SHA256SUMS.txt']) {
    const bytes = fs.readFileSync(path.join(directory, name));
    await api(`${release.upload_url.split('{')[0]}?name=${encodeURIComponent(name)}`, { method: 'POST', headers: { 'Content-Type': 'application/octet-stream' }, body: bytes });
  }
} catch (error) {
  throw new Error(`草稿已创建但附件上传未全部完成；未公开发布。请检查该草稿和本次 Actions 产物：${error.message}`);
}
console.log(`已创建草稿 ${tag}，构建提交 ${commit}；未公开发布。`);
if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `## Release 草稿已创建\n\n- 版本：${tag}\n- 提交：\`${commit}\`\n- [打开草稿](${release.html_url})\n- 未公开发布；请检查附件和人工验收后再决定发布。\n`);
