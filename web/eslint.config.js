import js from '@eslint/js';
import prettier from 'eslint-config-prettier';
import svelte from 'eslint-plugin-svelte';
import globals from 'globals';
import ts from 'typescript-eslint';

/**
 * The recommended sets of ESLint, typescript-eslint and the Svelte plugin, with Prettier owning the
 * formatting. Run by `pnpm lint` from scripts/check.sh; it must report nothing.
 */
export default ts.config(
	{ ignores: ['build/', '.svelte-kit/', 'node_modules/', 'src/lib/types/'] },
	js.configs.recommended,
	...ts.configs.recommended,
	...svelte.configs.recommended,
	prettier,
	...svelte.configs.prettier,
	{
		languageOptions: { globals: { ...globals.browser, ...globals.node } },
		rules: {
			// TypeScript checks names itself (typescript-eslint's advice).
			'no-undef': 'off',
			// ignoreRestSiblings: the `{ omitted, ...rest }` way to drop fields.
			'@typescript-eslint/no-unused-vars': [
				'error',
				{ argsIgnorePattern: '^_', varsIgnorePattern: '^_', ignoreRestSiblings: true }
			],
			// Off on purpose: the mustaches hold escapes and text Svelte would parse, the flagged links
			// are external or built hrefs (the app is served at /), and the flagged Sets are local to
			// pure helpers, never reactive state.
			'svelte/no-useless-mustaches': 'off',
			'svelte/no-navigation-without-resolve': 'off',
			'svelte/prefer-svelte-reactivity': 'off'
		}
	},
	{
		files: ['**/*.svelte', '**/*.svelte.ts', '**/*.svelte.js'],
		languageOptions: {
			parserOptions: {
				projectService: true,
				extraFileExtensions: ['.svelte'],
				parser: ts.parser
			}
		}
	}
);
