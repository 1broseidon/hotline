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
					label: 'Use Hotline',
					items: [
						{ label: 'Common workflows', slug: 'use/workflows' },
						{ label: 'Working in a conversation', slug: 'use/conversations' },
						{ label: 'Side threads and subagents', slug: 'use/side-threads' },
						{ label: 'Teammates working together', slug: 'use/teammates-together' },
						{ label: 'Calls and dictation', slug: 'use/voice' },
						{ label: 'Pictures', slug: 'use/images' },
					],
				},
				{
					label: 'Setup',
					items: [
						{ label: 'Providers and accounts', slug: 'setup/providers' },
						{ label: 'Teammates and drivers', slug: 'setup/teammates' },
						{ label: 'Tools, skills and web search', slug: 'setup/tools' },
						{ label: 'The computer', slug: 'setup/computer' },
						{ label: 'Schedules', slug: 'setup/schedules' },
						{ label: 'Updates', slug: 'setup/updates' },
					],
				},
				{
					label: 'Phone',
					items: [
						{ label: 'Hotline on your phone', slug: 'phone' },
						{ label: 'Link a desk', slug: 'phone/pair' },
						{ label: 'Use it away from home', slug: 'phone/away' },
						{ label: 'Talk to your teammates', slug: 'phone/chat' },
						{ label: 'Notifications', slug: 'phone/notifications' },
						{ label: "A teammate's computer", slug: 'phone/computer' },
						{ label: 'Updates', slug: 'phone/updates' },
					],
				},
				{
					label: 'Server',
					items: [
						{ label: 'Hotline on a server', slug: 'server' },
						{ label: 'Install', slug: 'server/install' },
						{ label: 'Pair your phone', slug: 'server/pair' },
						{ label: 'Link with your desktop', slug: 'server/link' },
						{ label: 'Run a relay', slug: 'server/relay' },
						{ label: 'Connect a model', slug: 'server/models' },
						{ label: 'Teammates and computers', slug: 'server/computers' },
						{ label: 'Day to day', slug: 'server/operate' },
						{ label: 'Troubleshooting', slug: 'server/troubleshooting' },
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
