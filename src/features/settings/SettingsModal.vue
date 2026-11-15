<script setup lang="ts">
import { computed, markRaw, ref, watch } from 'vue'
import { TabbedModal, ConfirmModal, Card, Button, DropdownSelect, Input, Checkbox } from '@modrinth/ui'
import { ArchiveIcon, SettingsIcon, LibraryIcon, HomeIcon, SearchIcon } from '@modrinth/assets'
import { useShellStore } from '@/stores/shell'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import type { CaptureSettings } from '@/platform/vault-types'

const shell = useShellStore()
const vault = useVaultStore()
const platform = usePlatform()
const capture = ref<CaptureSettings | null>(null)
const sourceText = ref('')
const processText = ref('')
const installText = ref('')
const extensionsText = ref('')
const configBusy = ref(false)
const settingsError = ref('')
const captureBaseline = ref('')
const captureDraft = computed(() => JSON.stringify([capture.value, sourceText.value, processText.value, installText.value, extensionsText.value]))
const captureDirty = computed(() => capture.value !== null && captureBaseline.value !== captureDraft.value)
async function loadCapture() {
	capture.value = null
	if (!vault.status?.dataRoot || platform.mode !== 'native') return
	try {
		capture.value = await platform.vault.getCaptureSettings()
		sourceText.value = capture.value.sources.map(source => source.path).join('\n')
		processText.value = capture.value.processNames.join(', ')
		installText.value = capture.value.wpsInstallPaths.join('\n')
		extensionsText.value = capture.value.extensions.join(', ')
		captureBaseline.value = captureDraft.value
	} catch (error) { settingsError.value = String(error) }
}
async function chooseRoot() {
	configBusy.value = true; settingsError.value = ''
	try { await platform.vault.chooseRoot(); await vault.refresh(platform); await loadCapture() } catch (error) { settingsError.value = String(error) }
	finally { configBusy.value = false }
}
async function saveCapture(): Promise<boolean> {
	if (!capture.value) return true
	configBusy.value = true; settingsError.value = ''
	const lines = (value: string) => value.split(/\r?\n/).map(part => part.trim()).filter(Boolean)
	try {
		await platform.vault.setCaptureSettings({ ...capture.value, sources: lines(sourceText.value).map(path => ({ path, recursive: true })), processNames: processText.value.split(/[,，\s]+/).filter(Boolean), wpsInstallPaths: lines(installText.value), extensions: extensionsText.value.split(/[,，\s]+/).filter(Boolean) })
		await vault.refresh(platform)
		await loadCapture()
		return true
	} catch (error) { settingsError.value = String(error); return false }
	finally { configBusy.value = false }
}
async function pause() { try { await platform.vault.pause(vault.status?.monitorState !== 'paused'); await vault.refresh(platform) } catch (error) { settingsError.value = String(error) } }
const modal = ref<InstanceType<typeof TabbedModal> | null>(null)
const discard = ref<InstanceType<typeof ConfirmModal> | null>(null)
const draftDirection = ref(shell.homeDirection)
const draftScale = ref(shell.scale)
const dirty = computed(() => captureDirty.value || draftDirection.value !== shell.homeDirection || draftScale.value !== shell.scale)
const tabs = [
	{ name: '存储与归档', category: '应用', icon: markRaw(ArchiveIcon) },
	{ name: '监测', category: '应用', icon: markRaw(SearchIcon) },
	{ name: '课程与课表', category: '应用', icon: markRaw(LibraryIcon) },
	{ name: '界面与行为', category: '应用', icon: markRaw(HomeIcon) },
	{ name: '数据与诊断', category: '应用', icon: markRaw(SettingsIcon) },
	{ name: '关于', category: 'CelesVault', icon: markRaw(ArchiveIcon) },
]
function resetDraft() { draftDirection.value = shell.homeDirection; draftScale.value = shell.scale }
function beforeHide() {
	if (!dirty.value) return true
	discard.value?.show()
	return false
}
function discardChanges() { resetDraft(); capture.value = null; modal.value?.hide() }
async function saveSession() {
	if (captureDirty.value && !(await saveCapture())) return
	shell.homeDirection = draftDirection.value
	shell.scale = draftScale.value
	document.documentElement.style.fontSize = `${16 * Number(draftScale.value.replace('%', '')) / 100}px`
	shell.statusMessage = '界面设置已应用到本次会话；持久配置将在后续阶段接入。'
	modal.value?.hide()
}
watch(() => shell.settingsOpen, open => {
	if (open) { resetDraft(); settingsError.value = ''; void loadCapture(); modal.value?.show() }
})
</script>

<template>
	<TabbedModal ref="modal" header="设置" :tabs="tabs" :before-hide="beforeHide" :on-hide="() => { shell.settingsOpen = false }" width="60rem" max-width="65rem" :floating-action-bar-shown="dirty">
		<template #content="{ index }">
			<div class="vault-setting">
				<p v-if="settingsError" class="vault-file-error text-sm" role="status">{{ settingsError }}</p>
				<template v-if="index === 0">
					<h2>存储与归档</h2>
					<Card><div class="vault-setting-label"><strong>持久目录</strong><small>{{ vault.status?.dataRoot ?? shell.runtime?.defaultDataRoot ?? '程序同级 data' }}</small><small>请选择不受还原影响的目录。初始化后开始真实监测；重启通过程序同级定位文件恢复。</small></div></Card>
					<Button type="outlined" :disabled="platform.mode !== 'native' || configBusy || dirty" @click="chooseRoot">{{ vault.status?.dataRoot ? '更换持久目录' : '选择持久目录并启动' }}</Button><small class="vault-muted">浏览器不会创建目录。更换目录不会迁移旧资料；有待处理归档或复制时禁止更换。</small>
				</template>
				<template v-else-if="index === 1">
					<h2>监测</h2>
					<Card><strong>{{ vault.status?.monitorState === 'running' ? '正在监测' : vault.status?.monitorState === 'paused' ? '已暂停' : vault.status?.monitorState === 'error' ? '监测异常' : '未配置' }}</strong><p class="vault-muted mt-2">{{ vault.status?.monitorDetail }}</p></Card>
					<Button type="outlined" :disabled="platform.mode !== 'native' || !vault.status?.dataRoot" @click="pause">{{ vault.status?.monitorState === 'paused' ? '恢复监测' : '暂停监测' }}</Button>
					<template v-if="capture">
						<DropdownSelect v-model="capture.mode" name="监测模式" aria-label="监测模式" :options="['loose', 'strict']" :display-name="value => value === 'loose' ? '宽松' : '严格'" />
						<Checkbox v-model="capture.includeDefaultSources" label="包含桌面、下载和 D 盘" /><Checkbox v-model="capture.includeUsb" label="包含已挂载 USB 存储" />
						<label>附加来源目录（每行一个，递归）<textarea v-model="sourceText" rows="3" class="w-full mt-2" /></label>
						<label>主程序名称<Input v-model="processText" class="mt-2" /></label>
						<label>WPS 安装目录（宽松模式辅助程序，每行一个）<textarea v-model="installText" rows="2" class="w-full mt-2" /></label>
						<label>扩展名<Input v-model="extensionsText" class="mt-2" /></label>
						<Button type="colored" color="brand" :disabled="configBusy" @click="saveCapture">保存监测规则</Button>
					</template>
					<p class="vault-muted">PID 只用于运行时关联。宽松也不是完美的人工打开/预览判断；不会把 Explorer 或 System 访问直接收纳。</p>
				</template>
				<template v-else-if="index === 2">
					<h2>课程与课表</h2>
					<Card><strong>时间段与星期课表分别编辑</strong><p class="vault-muted mt-2">课表在后续阶段接入，支持手动、CSV 与应用 JSON。当前不创建示例课表。</p></Card>
				</template>
				<template v-else-if="index === 3">
					<h2>界面与行为</h2>
					<div class="vault-setting-row"><div class="vault-setting-label"><label for="home-direction">主页分屏方向</label><small>本次会话使用，主页可随时切换。</small></div><DropdownSelect id="home-direction" name="主页分屏方向" aria-label="主页分屏方向" v-model="draftDirection" :options="['horizontal', 'vertical']" :display-name="id => id === 'horizontal' ? '左右' : '上下'" /></div>
					<div class="vault-setting-row"><div class="vault-setting-label"><label for="ui-scale">界面缩放</label><small>保持原控件比例；本次会话应用。</small></div><DropdownSelect id="ui-scale" name="界面缩放" aria-label="界面缩放" v-model="draftScale" :options="['100%', '110%', '125%']" /></div>
					<Card><strong>关闭到托盘</strong><p class="vault-muted mt-2">关闭窗口后可从托盘重新打开；退出使用托盘菜单。本机不注册自启动。</p></Card>
				</template>
				<template v-else-if="index === 4">
					<h2>数据与诊断</h2>
					<dl class="vault-diagnostics"><dt>运行模式</dt><dd>{{ shell.runtime?.mode === 'native' ? 'Tauri 原生宿主' : '浏览器预览' }}</dd><dt>管理员</dt><dd>{{ shell.runtime?.elevated ? '是' : '否（浏览器无原生权限）' }}</dd><dt>监测</dt><dd>{{ vault.status?.monitorState }}</dd><dt>归档待处理/失败</dt><dd>{{ vault.status?.pendingCount ?? 0 }} / {{ vault.status?.failedCount ?? 0 }}</dd><dt>持久设置</dt><dd>监测规则已接 data；界面设置仍仅本次会话</dd></dl>
					<p v-for="(line, index) in vault.status?.health ?? []" :key="index" class="vault-muted text-xs">{{ line }}</p>
					<p v-if="shell.statusMessage" class="vault-muted text-sm" role="status">最近操作：{{ shell.statusMessage }}</p>
					<Button type="outlined" disabled>导出诊断（尚未接入）</Button>
				</template>
				<template v-else>
					<h2>CelesVault</h2><p>版本 {{ shell.runtime?.version ?? '0.2.0' }} · 阶段 2</p>
					<p class="vault-muted">WPS 教学资料收纳与归档工具。当前是最小真实闭环，需 Native 验证；完整课表、固定夹和库尚未完成。</p>
					<Card><strong>第三方与许可</strong><p class="vault-muted mt-2">通用 UI 来自 Celestial Launcher；应用图标使用你的 CelesVault 图稿。应用按 GPL-3.0-only 分发，Inter 3.19 使用 SIL OFL。</p></Card>
				</template>
			</div>
		</template>
		<template #footer><p class="vault-muted px-4 py-3 text-xs">CelesVault {{ shell.runtime?.version ?? '0.2.0' }}<br />{{ shell.runtime?.mode === 'native' ? 'Windows · 原生宿主' : '浏览器预览' }}</p></template>
		<template #floating-action-bar>
			<Card v-if="dirty" class="!m-0"><div class="flex items-center justify-between gap-3"><span class="text-sm vault-muted">有未应用的修改</span><div class="flex gap-2"><Button type="outlined" @click="modal?.hide()">取消</Button><Button type="colored" color="brand" :disabled="configBusy" @click="saveSession">{{ captureDirty ? '保存规则并应用' : '应用到本次会话' }}</Button></div></div></Card>
		</template>
	</TabbedModal>
	<ConfirmModal ref="discard" title="放弃未应用的修改？" description="界面或监测规则尚未保存，关闭后丢弃本次草稿。" proceed-label="放弃修改" :danger="false" @proceed="discardChanges" />
</template>
