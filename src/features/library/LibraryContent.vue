<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { Input, Button, DropdownSelect, Checkbox, IconButton } from '@modrinth/ui'
import { GridIcon, ListIcon, XIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { useLibraryStore } from '@/stores/library'
import { usePlatform } from '@/platform'
import ArchiveTile from '@/components/ArchiveTile.vue'
import type { GroupBy, LibraryRow, SortBy } from '@/platform/vault-types'
const vault = useVaultStore()
const library = useLibraryStore()
const platform = usePlatform()
const from = computed({ get: () => library.query.dateFrom ?? '', set: value => { library.query.dateFrom = value || null } })
const to = computed({ get: () => library.query.dateTo ?? '', set: value => { library.query.dateTo = value || null } })
const groups = computed(() => {
	const values = new Map<string, { label: string; rows: LibraryRow[] }>()
	for (const row of library.result.rows) {
		const key = row.groupKey || 'all'
		if (!values.has(key)) values.set(key, { label: row.groupLabel, rows: [] })
		values.get(key)!.rows.push(row)
	}
	return [...values.entries()]
})
const groupNames: Record<GroupBy, string> = { none: '不分组', date: '日期', month: '年月', subject: '科目', course: '课次', type: '类型' }
const sortNames: Record<SortBy, string> = { archived: '收纳时间', used: '使用时间', name: '名称', size: '大小' }
const filtersOpen = ref(false)
let timer: ReturnType<typeof setTimeout> | undefined
function schedule() { if (timer) clearTimeout(timer); timer = setTimeout(() => { void library.load(platform) }, 180) }
watch(() => library.query, schedule, { deep: true })
watch(() => [library.query.name, library.query.regex, library.query.groupBy, library.query.sortBy, library.query.descending, library.query.subjects.join(','), library.query.extensions.join(','), library.query.dateFrom, library.query.dateTo], () => { library.query.offset = 0 })
watch(() => vault.status?.dataRoot, schedule)
function toggle(values: string[], id: string, selected: boolean) { return selected ? [...new Set([...values, id])] : values.filter(value => value !== id) }
function page(step: number) { library.query.offset = Math.max(0, library.query.offset + step * library.query.limit) }
onMounted(() => { void library.load(platform) })
onUnmounted(() => { if (timer) clearTimeout(timer); library.cancelPending() })
</script>
<template>
	<div class="vault-library-body">
		<header class="vault-library-controls"><h1>库</h1><span class="vault-muted text-sm">{{ library.result.total }} 项</span><div class="ml-auto flex gap-2"><IconButton label="网格视图" type="quiet" :aria-pressed="!library.listView" @click="library.listView = false"><GridIcon /></IconButton><IconButton label="列表视图" type="quiet" :aria-pressed="library.listView" @click="library.listView = true"><ListIcon /></IconButton><IconButton v-if="vault.libraryBrowsing" label="收起目录分屏" type="quiet" @click="vault.libraryBrowsing = false"><XIcon /></IconButton></div></header>
		<div class="vault-library-toolbar"><Input v-model="library.query.name" placeholder="搜索文件名" aria-label="库文件名搜索" class="grow" clearable /><Checkbox v-model="library.query.regex" label="正则" /></div>
		<div class="vault-library-filterbar"><DropdownSelect v-model="library.query.groupBy" name="资料分组" aria-label="资料分组" :options="Object.keys(groupNames)" :display-name="value => groupNames[value as GroupBy]" /><DropdownSelect v-model="library.query.sortBy" name="资料排序" aria-label="资料排序" :options="Object.keys(sortNames)" :display-name="value => sortNames[value as SortBy]" /><Button type="quiet" @click="library.query.descending = !library.query.descending">{{ library.query.descending ? '降序' : '升序' }}</Button><Button type="outlined" :aria-expanded="filtersOpen" @click="filtersOpen = !filtersOpen">筛选{{ library.query.subjects.length + library.query.extensions.length ? `（${library.query.subjects.length + library.query.extensions.length}）` : '' }}</Button></div>
		<div v-if="filtersOpen" class="vault-library-filters"><fieldset><legend>科目</legend><Checkbox v-for="subject in vault.config.timetable.subjects" :key="subject.id" :label="subject.name" :model-value="library.query.subjects.includes(subject.id)" @update:model-value="value => library.query.subjects = toggle(library.query.subjects, subject.id, value)" /><p v-if="!vault.config.timetable.subjects.length" class="vault-muted text-xs">尚未配置科目</p></fieldset><fieldset><legend>类型</legend><div class="vault-type-options"><Checkbox v-for="extension in vault.config.capture.extensions" :key="extension" :label="extension.toUpperCase()" :model-value="library.query.extensions.includes(extension)" @update:model-value="value => library.query.extensions = toggle(library.query.extensions, extension, value)" /></div></fieldset><div class="flex flex-wrap gap-2"><Input v-model="from" placeholder="开始日期 YYYY-MM-DD" aria-label="起始日期" /><Input v-model="to" placeholder="结束日期 YYYY-MM-DD" aria-label="截止日期" /><Button type="quiet" @click="library.query.subjects = []; library.query.extensions = []; from = ''; to = ''">清除筛选</Button></div></div>
		<p v-if="library.error" class="vault-file-error text-sm" role="status">{{ library.error }}</p>
		<div class="vault-library-records"><p v-if="library.loading" class="vault-muted">查询中…</p><section v-for="[key, group] in groups" :key="key"><h2 v-if="library.query.groupBy !== 'none'">{{ group.label }}</h2><div class="vault-file-grid" :class="{ list: library.listView }"><ArchiveTile v-for="row in group.rows" :key="`${key}-${row.item.id}`" :item="row.item" :compact="library.listView" /></div></section><div v-if="!library.loading && !library.result.rows.length" class="vault-empty"><strong>没有匹配资料</strong><p>按实体展示，使用历史可在详情查看；只匹配名称，不读取全文。</p></div></div>
		<footer class="vault-library-pagination"><Button :disabled="library.query.offset === 0 || library.loading" @click="page(-1)">上一页</Button><span>{{ Math.floor(library.query.offset / library.query.limit) + 1 }} / {{ Math.max(1, Math.ceil(library.result.total / library.query.limit)) }}</span><Button :disabled="library.query.offset + library.query.limit >= library.result.total || library.loading" @click="page(1)">下一页</Button></footer>
	</div>
</template>
