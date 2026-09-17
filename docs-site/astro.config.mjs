// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
	site: 'https://hotline.dev',
	base: '/docs',
	outDir: '../site/public/docs',
	integrations: [
		starlight({
			title: 'Hotline',
			description: 'A local-first room for your team of agents. Open source, self-hosted, on your machine.',
			logo: { src: './src/assets/hotline-mark.svg', alt: 'Hotline' },
			favicon: '/favicon.svg',
			social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/1Broseidon/hotline' }],
			editLink: { baseUrl: 'https://github.com/1Broseidon/hotline/edit/main/docs-site/' },
			customCss: ['./src/styles/hotline.css'],
			head: [
				{ tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.googleapis.com' } },
				{ tag: 'link', attrs: { rel: 'preconnect', href: 'https://fonts.gstatic.com', crossorigin: true } },
				{
					tag: 'link',
					attrs: {
						rel: 'stylesheet',
						href: 'https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600&family=IBM+Plex+Mono:wght@400;500&display=swap',
					},
				},
			],
			sidebar: [
				{
					label: 'Get started',
					items: [
						{ label: 'Install', slug: 'get-started/install' },
						{ label: 'Your first teammate', slug: 'get-started/first-teammate' },
						{ label: 'How a room works', slug: 'get-started/room' },
					],
				},
				{
					label: 'Setup',
					items: [
						{ label: 'Providers and keys', slug: 'setup/providers' },
						{ label: 'Teammates and drivers', slug: 'setup/teammates' },
						{ label: 'Tools and MCP servers', slug: 'setup/tools' },
						{ label: 'The computer', slug: 'setup/computer' },
						{ label: 'Schedules', slug: 'setup/schedules' },
						{ label: 'Updates', slug: 'setup/updates' },
					],
				},
				{
					label: 'Reference',
					items: [
						{ label: 'Keyboard shortcuts', slug: 'reference/shortcuts' },
						{ label: 'Data and privacy', slug: 'reference/data' },
						{ label: 'Import a previous edition', slug: 'reference/import' },
					],
				},
			],
		}),
	],
});
