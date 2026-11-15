<script setup lang="ts">
import { Card } from '@modrinth/ui'
import { LibraryIcon } from '@modrinth/assets'
</script>

<template>
	<section class="vault-page" aria-labelledby="library-title">
		<header class="vault-page-heading"><h1 id="library-title">库</h1><span class="vault-muted text-sm">资料与使用历史</span></header>
		<Card class="vault-panel grow">
			<div class="vault-empty">
				<LibraryIcon />
				<strong>资料库尚未接入</strong>
				<p>这个路由保留全宽浏览布局。真实索引、分组、筛选和全局搜索在后续阶段接入；当前没有虚构资料。</p>
			</div>
		</Card>
	</section>
</template>
