import { logoUrl } from '$lib/api';

/**
 * The stored logo's image (`GET /api/settings/branding/logo`), for the Branding page and the
 * email previews only: `meta`, read on every page, does not carry it, and an `<img>` loads it
 * only where one is shown. The browser revalidates it by its entity tag on each page load; within
 * one, a branding save calls `changed()` so the next `url` asks the server again.
 */
class StoredLogo {
	#revision = $state(0);

	/** The image's URL when a logo is stored (`has_logo` from the settings), else `null`. */
	url(hasLogo: boolean): string | null {
		return hasLogo ? logoUrl(this.#revision) : null;
	}

	/** After a logo upload or removal. */
	changed(): void {
		this.#revision++;
	}
}

export const storedLogo = new StoredLogo();
