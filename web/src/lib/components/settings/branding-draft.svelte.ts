import { deleteLogo, previewBranding, saveBranding, uploadLogo } from '$lib/api';
import { formatInt } from '$lib/format';
import { storedLogo } from '$lib/state/logo.svelte';
import { meta } from '$lib/state/meta.svelte';
import { settings } from '$lib/state/settings.svelte';
import type { BrandingPreviewRequest } from '$lib/types/BrandingPreviewRequest';
import type { BrandingSettings } from '$lib/types/BrandingSettings';
import type { EmailPreview } from '$lib/types/EmailPreview';
import type { LogoType } from '$lib/types/LogoType';
import type { PixelSize } from '$lib/types/PixelSize';
import type { SettingsResponse } from '$lib/types/SettingsResponse';
import { invalid, SettingsForm, type Invalid, type SettingsFormSource, type Writes } from './settings-form.svelte';

/**
 * What happens to the logo on save: nothing, a picked image replaces it, or it is removed. A
 * picked image carries its size in pixels as the browser read it (`null` until read), for the
 * preview: the server sizes the logo in the emails from it.
 */
export type LogoChange =
	{ kind: 'keep' } | { kind: 'replace'; file: File; size: PixelSize | null } | { kind: 'remove' };

/** The editable fields of the branding group: the operator name, and what happens to the logo. */
export type BrandingDraft = BrandingSettings & { logo: LogoChange };

/** The image types a logo may be; the server checks the bytes again. */
export const LOGO_ACCEPT = 'image/png,image/jpeg,image/webp';

/** A stored logo's format in words. */
export const LOGO_TYPE_LABEL: Record<LogoType, string> = { png: 'PNG', jpeg: 'JPEG', webp: 'WebP' };

/** `bytes` in whole kilobytes, rounded up like the server's message ("1 KB" for anything below). */
export function kilobytes(bytes: number): string {
	return `${formatInt(Math.max(1, Math.ceil(bytes / 1024)))} KB`;
}

/**
 * Why a draft cannot be saved, worded like the server's own check (which reads the image's bytes
 * and checks again). `maxBytes` is the server's limit (`meta.logo_max_bytes`); without it only
 * the type is checked here. Client-side because it is shown as soon as a file is picked and holds
 * Save, so a file the server would refuse is never uploaded.
 */
export function problemWith(d: BrandingDraft, maxBytes: number | null): string | null {
	if (d.logo.kind !== 'replace') return null;
	const file = d.logo.file;
	if (!LOGO_ACCEPT.split(',').includes(file.type)) {
		return 'The logo must be a PNG, JPEG or WebP image.';
	}
	if (file.size === 0) return 'The logo file is empty.';
	if (maxBytes !== null && file.size > maxBytes) {
		return `The logo is ${kilobytes(file.size)}; it can be at most ${kilobytes(maxBytes)}. Save it smaller or as a JPEG or WebP.`;
	}
	return null;
}

/**
 * The draft as the preview asks for it: the name as typed, whether a logo would top the email and
 * a picked logo's size (the stored logo's is the server's); `null` while a picked logo's size is
 * not read yet.
 */
export function previewRequest(d: BrandingDraft, s: SettingsResponse): BrandingPreviewRequest | null {
	const name = d.operator_name.trim();
	if (d.logo.kind === 'replace') {
		return d.logo.size ? { operator_name: name, has_logo: true, logo_size: d.logo.size } : null;
	}
	return { operator_name: name, has_logo: d.logo.kind === 'keep' && s.branding.has_logo };
}

/** `file`'s size in pixels as the browser decodes it; `null` when it cannot. */
export async function imageSize(file: File): Promise<PixelSize | null> {
	if (typeof createImageBitmap !== 'function') return null;
	try {
		const bitmap = await createImageBitmap(file);
		const size = { width: bitmap.width, height: bitmap.height };
		bitmap.close();
		return size;
	} catch {
		return null;
	}
}

/** How long typing pauses before the preview asks again. */
export const PREVIEW_DELAY_MS = 250;

/**
 * The line under the preview: without a name, that the emails go out as Marcan; else whether they
 * go out with the free-software notice (the server's `notice_shown`: Marcan opens them, the
 * operator's letterhead second) or licensed (the operator's letterhead alone).
 */
export function previewNote(p: EmailPreview): 'no_name' | 'notice' | 'licensed' {
	if (p.operator_name === null) return 'no_name';
	return p.notice_shown ? 'notice' : 'licensed';
}

/**
 * The Branding page's model (the branding group): the operator name and the logo change (a
 * picked image, or "Remove logo"), saved together by the save bar through `settings.save`: the
 * name, then the upload or removal, after which the stored logo shown is asked for again.
 * Saving never reads the license; `preview` (the server's "How your emails look" for the draft,
 * asked again once typing pauses) shows the emails as the stored license makes them, and in the
 * other license state. Create it during component initialisation (it owns effects).
 */
export class BrandingModel extends SettingsForm<SettingsResponse, BrandingDraft> {
	static readonly writes: Writes = { page: 'branding', groups: ['branding'] };
	/** How the emails look with the draft (the server's preview); `null` until the first answer. */
	preview: EmailPreview | null = $state(null);
	#previewSeq = 0;

	constructor(options: { source: SettingsFormSource<SettingsResponse> }) {
		super({ ...options.source, label: 'Branding', blank: { operator_name: '', logo: { kind: 'keep' } } });
		// The preview follows the draft, asked again once typing pauses; a refused name keeps the last one.
		$effect(() => {
			const saved = this.saved;
			if (!saved) return;
			const request = previewRequest(this.normalized, saved);
			if (!request) return;
			const timer = setTimeout(() => this.loadPreview(request), PREVIEW_DELAY_MS);
			return () => clearTimeout(timer);
		});
	}

	protected toDraft(s: SettingsResponse): BrandingDraft {
		return { operator_name: s.branding.operator_name, logo: { kind: 'keep' } };
	}

	protected normalize(d: BrandingDraft): BrandingDraft {
		return { operator_name: d.operator_name.trim(), logo: d.logo };
	}

	protected check(d: BrandingDraft): Invalid | null {
		return invalid('logo', problemWith(d, meta.current?.logo_max_bytes ?? null));
	}

	async loadPreview(request: BrandingPreviewRequest): Promise<void> {
		const seq = ++this.#previewSeq;
		try {
			const preview = await previewBranding(request);
			if (seq === this.#previewSeq) this.preview = preview;
		} catch {
			// Kept: the last preview stands until the draft can be previewed again.
		}
	}

	/** Use `file` as the logo on save (a draft change), its size read for the preview. */
	pick(file: File | null | undefined): void {
		this.draft.logo = file ? { kind: 'replace', file, size: null } : { kind: 'keep' };
		if (!file) return;
		imageSize(file).then((size) => {
			const logo = this.draft.logo;
			if (size && logo.kind === 'replace' && logo.file === file) this.draft.logo = { ...logo, size };
		});
	}

	/** Remove the stored logo on save, or forget a picked one (a draft change). */
	removeLogo(): void {
		this.draft.logo = this.saved?.branding.has_logo ? { kind: 'remove' } : { kind: 'keep' };
	}

	protected async write(d: BrandingDraft): Promise<{ saved: SettingsResponse }> {
		try {
			let saved = await settings.save(saveBranding({ operator_name: d.operator_name }));
			if (d.logo.kind === 'replace') saved = await settings.save(uploadLogo(d.logo.file));
			else if (d.logo.kind === 'remove') saved = await settings.save(deleteLogo());
			return { saved };
		} finally {
			if (d.logo.kind !== 'keep') storedLogo.changed();
		}
	}
}
