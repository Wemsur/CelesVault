import type { Subject } from './vault-types'

// 固定柔和色组；旧配置不必先保存便可按科目ID取得稳定颜色。
const palette = ['#7ba6d9', '#bb91d4', '#d6a46f', '#79b6a4', '#ce8798', '#90a0d2', '#b5b975', '#7bb8c8', '#b69b89', '#98b88c']
export function subjectColor(id: string, configured?: string | null, subjects: Subject[] = []): string {
	if (configured && /^#[0-9a-f]{6}$/i.test(configured)) return configured
	const used = new Set(subjects.flatMap(subject => subject.color ? [subject.color.toLowerCase()] : []))
	for (const subjectId of [...new Set([...subjects.filter(subject => !subject.color).map(subject => subject.id), id])].sort()) {
		let hash = 0
		for (const char of subjectId) hash = (Math.imul(hash, 31) + char.charCodeAt(0)) >>> 0
		const colors = [...palette.slice(hash % palette.length), ...palette.slice(0, hash % palette.length)]
		let color = colors.find(color => !used.has(color))
		if (!color) {
			let value = hash & 0xffffff
			do { color = `#${value.toString(16).padStart(6, '0')}`; value = (value + 7919) & 0xffffff } while (used.has(color))
		}
		if (subjectId === id) return color
		used.add(color)
	}
	return palette[0]
}
export function nextSubjectColor(subjects: Subject[]): string {
	const used = new Set(subjects.map(subject => subjectColor(subject.id, subject.color, subjects).toLowerCase()))
	return palette.find(color => !used.has(color)) ?? palette[subjects.length % palette.length]
}
