export const backgroundIds = ['aurora','silk','mesh','grain','ice','veil','smoke','neuro','flow','rays','blend','flux','gate'] as const
export type BackgroundId = typeof backgroundIds[number]
export type ThemeMode = 'customdark' | 'customlight' | 'system'
export interface BackgroundParameters { quality: number; speed: number; intensity: number; fps: number; scale: number; detail: number; grain: number; density: number; shockwave: boolean }
export interface AppearanceSettings {
	theme: ThemeMode; hue: number; background: 'off' | 'animated' | 'image'; shader: BackgroundId
	parameters: Record<BackgroundId, BackgroundParameters>
	image: string | null; imageFit: 'cover' | 'contain'; imageOpacity: number; imageBlur: number
}
export const defaultBackgroundParameters = (): BackgroundParameters => ({quality: .5, speed: 1, intensity: .6, fps: 30, scale: 1, detail: 1, grain: 1, density: 80, shockwave: true})
export const defaultAppearance = (): AppearanceSettings => ({theme:'customdark',hue:38,background:'off',shader:'aurora',parameters:Object.fromEntries(backgroundIds.map(id=>[id,defaultBackgroundParameters()])) as Record<BackgroundId, BackgroundParameters>,image:null,imageFit:'cover',imageOpacity:.6,imageBlur:0})
