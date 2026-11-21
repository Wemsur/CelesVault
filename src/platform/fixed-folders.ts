import type { DirectoryView, DriveInfo, FixedFolder } from './vault-types'

export function favoriteDeviceKey(drive: DriveInfo): string | null {
	if (drive.identityReliable && drive.deviceId) return drive.deviceId
	return drive.isUsb ? null : `local-volume:${drive.volumeId}`
}
export function favoriteLocation(path: string, volumeId: string, drives: DriveInfo[]) {
	const drive = drives.find(item => item.volumeId === volumeId && path.toLowerCase().startsWith(item.root.toLowerCase()))
	if (!drive) throw new Error('文件夹对应的分区已移除或发生变化，请刷新目录')
	const deviceId = favoriteDeviceKey(drive)
	if (!deviceId) throw new Error('设备身份不可靠，无法固定此文件夹')
	return { drive, deviceId, volumeId, relativePath: path.slice(drive.root.length).replaceAll('\\','/') }
}
export function matchingFavorite(favorites: FixedFolder[], path: string, volumeId: string, drives: DriveInfo[]) {
	try {
		const location = favoriteLocation(path, volumeId, drives)
		return favorites.find(folder => folder.deviceId === location.deviceId && folder.volumeId === location.volumeId && folder.relativePath.replaceAll('\\','/').toLowerCase() === location.relativePath.toLowerCase())
	} catch { return undefined }
}
export type FolderPinSource = Pick<DirectoryView, 'path' | 'identity' | 'volumeId'>
