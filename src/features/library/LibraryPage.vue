<script setup lang="ts">
import { computed, onActivated, onMounted, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useVaultStore } from '@/stores/vault'
import { useLibraryStore } from '@/stores/library'
import { usePlatform } from '@/platform'
import ResizableSplit from '@/components/ResizableSplit.vue'
import DirectoryBrowser from '@/features/file-browser/DirectoryBrowser.vue'
import LibraryContent from './LibraryContent.vue'
const route = useRoute()
const vault = useVaultStore()
const library = useLibraryStore()
const platform = usePlatform()
const ratio = computed({ get: () => vault.config.app.libraryRatio, set: value => { vault.config.app.libraryRatio = value } })
watch(() => route.query, () => { if (route.name === 'library') library.search(String(route.query.q ?? ''), route.query.regex === '1') }, { immediate: true })
function enter() { vault.targetContext = 'library'; vault.libraryBrowsing = false }
onMounted(enter)
onActivated(enter)
</script>
<template>
	<section class="vault-library-page"><ResizableSplit v-model="ratio" :single="!vault.libraryBrowsing" :min-size="260" @commit="value => vault.persistApp(platform, { libraryRatio: value })"><template #first><LibraryContent /></template><template #second><DirectoryBrowser context="library" /></template></ResizableSplit></section>
</template>
