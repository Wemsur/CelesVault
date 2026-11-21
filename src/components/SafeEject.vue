<script setup lang="ts">
import { computed, ref } from 'vue'
import { Button, NewModal } from '@modrinth/ui'
import { UnplugIcon } from '@modrinth/assets'
import { useVaultStore } from '@/stores/vault'
import { usePlatform } from '@/platform'
const vault=useVaultStore();const platform=usePlatform()
const modal=ref<InstanceType<typeof NewModal>|null>(null);const error=ref('');const busy=ref(false)
const ejectable=computed(()=>[...new Map(vault.drives.filter(drive=>drive.canEject&&drive.deviceId).map(drive=>[drive.deviceId,drive])).values()])
async function eject(id:string){busy.value=true;error.value='';let done=false;try{await platform.files.unwatch();await platform.files.eject(id);await vault.refresh(platform);done=true}catch(e){error.value=String(e)}finally{busy.value=false}if(done)modal.value?.hide()}
</script>
<template>
	<Button class="vault-eject" type="outlined" :disabled="!ejectable.length||platform.mode!=='native'" :title="ejectable.length?'安全弹出外接存储':'当前没有可安全弹出的外接设备'" @click="modal?.show()"><UnplugIcon/>安全弹出</Button>
	<NewModal ref="modal" header="安全弹出外接设备" width="28rem" :disable-close="busy"><p class="vault-muted">活动归档、复制或data正在使用该设备时会拒绝。不会强制卸载或弹出系统盘。</p><Button v-for="drive in ejectable" :key="drive.identity" type="outlined" class="w-full mt-3" :disabled="busy" @click="eject(drive.deviceId!)">{{drive.name}}</Button><p v-if="error" class="vault-file-error mt-3" role="status">{{error}}</p></NewModal>
</template>
