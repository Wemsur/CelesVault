<script setup lang="ts">
import { ref } from 'vue'
import { Card, Button, IconButton, Input, Checkbox, Toggle, DropdownSelect, Combobox, ContextMenu, TeleportOverflowMenu, NewModal, ConfirmModal, type ButtonMenuOption } from '@modrinth/ui'
import { PlusIcon, XIcon } from '@modrinth/assets'
const selected = ref(true)
const toggled = ref(false)
const name = ref('这是一份用于检查两行省略和长文本控件布局的资料名称，不是归档文件.docx')
const sort = ref('名称')
const modal = ref<InstanceType<typeof NewModal> | null>(null)
const confirm = ref<InstanceType<typeof ConfirmModal> | null>(null)
const result = ref('')
const contextMenu = ref<InstanceType<typeof ContextMenu> | null>(null)
const comboChoice = ref('docx')
const comboOptions = [
	{ value: 'docx', label: 'Word 文档' },
	{ value: 'pdf', label: 'PDF 文档' },
	{ value: 'jpg', label: '图片', disabled: true },
]
const menuOptions: ButtonMenuOption[] = [
	{ id: 'test', label: '更新测试文字', action: () => { result.value = '菜单动作已执行，没有文件操作。' } },
	{ id: 'disabled', label: '禁用操作', disabled: true },
	{ type: 'divider' },
	{ type: 'submenu', id: 'view', label: '子菜单', options: [{ id: 'nested', label: '子菜单测试', action: () => { result.value = '子菜单动作已执行。' } }] },
]
</script>

<template>
	<section class="vault-page">
		<header><h1>原控件展示</h1><p class="vault-muted mt-2">仅开发路由 · 状态验证数据不写入正式资料库。</p></header>
		<Card>
			<h2 class="mb-4">按钮与状态</h2>
			<div class="vault-showcase"><Button>普通按钮</Button><Button type="colored" color="brand">主要按钮</Button><Button type="outlined">线框按钮</Button><Button type="quiet">轻量按钮</Button><Button disabled>禁用</Button><Button loading>加载中</Button><IconButton label="添加"><PlusIcon /></IconButton><IconButton label="关闭" disabled><XIcon /></IconButton></div>
		</Card>
		<Card>
			<h2 class="mb-4">输入、选择与下拉</h2>
			<div class="vault-showcase"><Input v-model="name" aria-label="长文本测试" clearable /><Input model-value="禁用输入" disabled aria-label="禁用输入" /><label class="flex items-center gap-2"><Checkbox v-model="selected" />选中状态</label><label class="flex items-center gap-2"><Toggle v-model="toggled" />启用状态</label><DropdownSelect name="排序方式" aria-label="排序方式" v-model="sort" :options="['名称', '收纳时间', '文件大小', '很长的下拉选项用于检查浮层是否保留主题和截断']" /></div>
		</Card>
		<Card><h2 class="mb-4">原菜单与浮层</h2><div class="vault-showcase"><Combobox v-model="comboChoice" :options="comboOptions" searchable aria-label="文件类型测试" /><TeleportOverflowMenu label="更多操作" :options="menuOptions"><PlusIcon /></TeleportOverflowMenu><Button @contextmenu.prevent="contextMenu?.open($event, menuOptions)">右键打开原菜单</Button></div></Card>
		<ContextMenu ref="contextMenu" label="控件测试菜单" />
		<Card><h2 class="mb-4">两行长名与弹窗</h2><p class="vault-long-name mb-4">{{ name }}</p><div class="vault-showcase"><Button @click="modal?.show()">滚动弹窗</Button><Button type="outlined" @click="confirm?.show()">确认弹窗</Button><span role="status">{{ result }}</span></div></Card>
		<NewModal ref="modal" header="原弹窗滚动状态" max-width="38rem" scrollable><p v-for="line in 30" :key="line" class="mb-4">第 {{ line }} 行：这里是明确的开发控件测试文案，用于确认滚动、焦点、边缘指示与 Teleport 主题。</p><template #actions><Button @click="modal?.hide()">关闭</Button></template></NewModal>
		<ConfirmModal ref="confirm" title="确认控件测试" description="此操作只更新展示页文字，不修改文件。" proceed-label="确认" :danger="false" @proceed="result = '控件确认已执行，没有文件操作。'" />
	</section>
</template>
