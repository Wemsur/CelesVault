import { spawn, execFileSync } from 'node:child_process';
import { createHash, createHmac, randomBytes } from 'node:crypto';
import { lstatSync, mkdirSync, readFileSync, readdirSync, realpathSync, unlinkSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:net';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const hex64 = /^[0-9a-f]{64}$/;
const hex32 = /^[0-9a-f]{32}$/;
const instanceName = /^([1-9][0-9]{0,9})-([1-9][0-9]{0,19})\.json$/;
// Vite 默认拒绝从开发服务器提供 .env.* 文件。
const ticketName = '.env.celesvault-ticket';
const require = createRequire(import.meta.url);

function mac(token, kind, session, { pid, created, exe }) {
	return createHmac('sha256', token).update(`${kind}\n${session}\n${pid}\n${created}\n${exe}`).digest('hex');
}

function statIfPresent(file) {
	try { return lstatSync(file); } catch (error) {
		if (error.code !== 'ENOENT') throw error;
		return null;
	}
}

function readSmall(file, limit) {
	const info = statIfPresent(file);
	return info?.isFile() && info.size <= limit ? readFileSync(file, 'utf8') : null;
}

function ensureDirectory(dir) {
	try { mkdirSync(dir); } catch (error) { if (error.code !== 'EEXIST') throw error; }
	if (!lstatSync(dir).isDirectory()) throw new Error(`不使用链接或非目录作为开发握手目录：${dir}`);
}

function sameImage(candidate, expected) {
	// Rust canonicalize 使用扩展 Windows 路径；普通 Win32 路径和它等价。
	if (typeof candidate !== 'string') return false;
	const plain = candidate.startsWith('\\\\?\\UNC\\') ? `\\\\${candidate.slice(8)}`
		: candidate.startsWith('\\\\?\\') ? candidate.slice(4) : candidate;
	if (plain.toLowerCase() !== expected.toLowerCase()) return false;
	return statIfPresent(plain)?.isFile() && realpathSync.native(plain) === expected;
}

function processStatus(record, context) {
	const result = execFileSync(context.powershell, ['-NoProfile', '-NonInteractive', '-EncodedCommand', context.query], {
		cwd: context.root, windowsHide: true, encoding: 'utf8', timeout: 4000, maxBuffer: 32 * 1024,
		stdio: ['ignore', 'pipe', 'pipe'],
		env: { ...process.env, CELESVAULT_QUERY_PID: String(record.pid), CELESVAULT_QUERY_CREATED: record.created },
	});
	const data = JSON.parse(result);
	if (data.status === 'gone' || data.status === 'reused') return data.status;
	if (data.status !== 'present' || typeof data.exe !== 'string') throw new Error('无法确认 Windows 进程身份。');
	return sameImage(data.exe, context.expected) ? 'owned' : 'foreign';
}

function forget(file) {
	try { unlinkSync(file); } catch (error) {
		if (error.code !== 'ENOENT') console.warn(`无法移除过期开发记录 ${file}：${error.message}`);
	}
}

async function retire(session, dir, file, record, token, context) {
	const request = path.join(dir, `${record.pid}-${record.created}.exit`);
	const proof = mac(token, 'exit-v1', session, record);
	try {
		const initial = processStatus(record, context);
		if (initial === 'gone' || initial === 'reused') {
			forget(file);
			forget(request);
			return;
		}
		if (initial !== 'owned') return;

		// 刚确认身份后仅发布定向请求，不发送窗口消息或结束进程。
		try { writeFileSync(request, proof, { flag: 'wx' }); } catch (error) {
			if (error.code !== 'EEXIST' || readSmall(request, 64) !== proof) throw error;
		}
		console.log(`已请求同一项目的开发实例正常退出（PID ${record.pid}）。`);
		const deadline = performance.now() + 20_000;
		while (performance.now() < deadline) {
			await delay(500);
			const status = processStatus(record, context);
			if (status === 'gone' || status === 'reused') {
				forget(file);
				forget(request);
				return;
			}
			if (status !== 'owned') throw new Error('等待期间无法再次确认该实例，已停止。');
		}
		throw new Error('等待正常退出超时；退出请求可能仍在处理，未启动替代实例。');
	} catch (error) {
		throw new Error(`PID ${record.pid}（${context.expected}）：${error.message}。请核对旧开发终端或对应实例的托盘退出。`);
	}
}

async function cleanup(store, context) {
	for (const entry of readdirSync(store, { withFileTypes: true })) {
		const session = entry.name;
		if (!entry.isDirectory() || !hex32.test(session)) continue;
		const dir = path.join(store, session);
		const token = readSmall(path.join(dir, ticketName), 64);
		if (!token || !hex64.test(token)) continue;
		for (const name of readdirSync(dir)) {
			const match = instanceName.exec(name);
			if (!match) continue;
			const file = path.join(dir, name);
			let record;
			try { record = JSON.parse(readSmall(file, 65536)); } catch { continue; }
			if (!record || record.pid !== Number(match[1]) || record.pid > 0x7fffffff || record.created !== match[2]
				|| typeof record.exe !== 'string' || record.proof !== mac(token, 'register-v1', session, record)
				|| !sameImage(record.exe, context.expected)) continue;
			await retire(session, dir, file, record, token, context);
		}
	}
}

async function main() {
	if (process.platform !== 'win32') throw new Error('CelesVault Native 开发启动器仅支持 Windows。');
	if (process.argv.length !== 2) throw new Error('此开发入口不接受自定义 Tauri 参数，仅支持默认 debug 目标。');
	const root = realpathSync.native(fileURLToPath(new URL('..', import.meta.url)));
	const cli = require.resolve('@tauri-apps/cli/tauri.js');
	// 只约束并发启动器；不通过管道传递命令，也不把占用当作进程归属证明。
	const lockName = `\\\\.\\pipe\\celesvault-dev-${createHash('sha256').update(root.toLowerCase()).digest('hex')}`;
	const lock = createServer((socket) => socket.destroy());
	await new Promise((resolve, reject) => {
		lock.once('error', reject);
		lock.listen(lockName, resolve);
	}).catch((error) => {
		if (error.code === 'EADDRINUSE') throw new Error('本项目已有开发启动器运行，请先停止旧开发终端。');
		throw error;
	});
	try {
		const target = path.join(root, 'target');
		const store = path.join(target, 'celesvault-dev');
		ensureDirectory(target);
		ensureDirectory(store);
		const debug = path.join(target, 'debug');
		const debugInfo = statIfPresent(debug);
		if (debugInfo && !debugInfo.isDirectory()) throw new Error(`不使用链接或非目录作为 debug 目录：${debug}`);
		const exe = path.join(debug, 'celesvault.exe');
		const exeInfo = statIfPresent(exe);
		if (exeInfo && !exeInfo.isFile()) throw new Error(`不使用链接或非文件作为 debug EXE：${exe}`);
		if (exeInfo) {
			const context = {
				root, expected: realpathSync.native(exe),
				powershell: path.join(process.env.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe'),
				query: Buffer.from(readFileSync(new URL('./dev-process-metadata.ps1', import.meta.url), 'utf8'), 'utf16le').toString('base64'),
			};
			await cleanup(store, context);
		}
		const session = randomBytes(16).toString('hex');
		const token = randomBytes(32).toString('hex');
		const dir = path.join(store, session);
		mkdirSync(dir);
		writeFileSync(path.join(dir, ticketName), token, { flag: 'wx' });
		// 同一控制台中 Tauri 自己处理 Ctrl+C；包装器不遍历或终止进程树。
		const onInterrupt = () => {};
		process.on('SIGINT', onInterrupt);
		try {
			process.exitCode = await new Promise((resolve, reject) => {
				const child = spawn(process.execPath, [cli, 'dev'], {
					cwd: root, stdio: 'inherit',
					env: { ...process.env, CELESVAULT_DEV_SESSION: session, CELESVAULT_DEV_TOKEN: token },
				});
				child.once('error', reject);
				child.once('exit', (code, signal) => resolve(code ?? (signal === 'SIGINT' ? 130 : 1)));
			});
		} finally {
			process.off('SIGINT', onInterrupt);
		}
	} finally {
		lock.close();
	}
}

main().catch((error) => {
	console.error(`CelesVault 开发启动失败：${error.message}`);
	process.exitCode = 1;
});
