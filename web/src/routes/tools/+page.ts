import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

// The DNS Inspector is the only tool; keep old /tools links working.
export const load: PageLoad = () => {
	redirect(307, '/tools/dns');
};
