<script setup lang="ts">
import { computed, onActivated, onMounted } from 'vue'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
import ResizableSplit from '@/components/ResizableSplit.vue'
import DirectoryBrowser from '@/features/file-browser/DirectoryBrowser.vue'
import LibraryContent from './LibraryContent.vue'
const vault = useVaultStore()
const platform = usePlatform()
const ratio = computed({ get: () => vault.config.app.libraryRatio, set: value => { vault.config.app.libraryRatio = value } })
function enter() { vault.targetContext = 'library'; vault.libraryBrowsing = false }
onMounted(enter)
onActivated(enter)
</script>
<template>
	<section class="vault-library-page"><ResizableSplit v-model="ratio" :single="!vault.libraryBrowsing" :min-size="260" @commit="value => vault.persistApp(platform, { libraryRatio: value })"><template #first><LibraryContent /></template><template #second><DirectoryBrowser context="library" /></template></ResizableSplit></section>
</template>
