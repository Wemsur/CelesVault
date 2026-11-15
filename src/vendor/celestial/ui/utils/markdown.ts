// CelesVault safety adaptation, 2026-11-14. No network/image proxy or shared-utils barrel.
import DOMPurify from 'dompurify'
import { marked } from 'marked'

/** https://marked.js.org/#usage; https://github.com/cure53/DOMPurify#can-i-configure-dompurify */
export function renderString(value: string): string {
	if (!DOMPurify.isSupported) {
		return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
	}
	const html = marked.parse(value, { async: false, gfm: true, breaks: true })
	return DOMPurify.sanitize(html, {
		USE_PROFILES: { html: true },
		FORBID_TAGS: [
			'script', 'style', 'iframe', 'object', 'embed', 'img', 'picture',
			'source', 'video', 'audio', 'link', 'form', 'input', 'button',
		],
		FORBID_ATTR: ['style', 'src', 'srcset', 'poster', 'background'],
	})
}
