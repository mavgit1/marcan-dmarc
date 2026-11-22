import type { DomainOrganization } from '#lib/types/DomainOrganization.ts';

/**
 * Page-level metadata published by route components for global UI like AppHeader: the entity's
 * name, and on a domain page the organization it belongs to (the trail's middle step).
 */
export const pageMeta = $state<{
	entityName: string | null;
	organization: DomainOrganization | null;
}>({
	entityName: null,
	organization: null
});
