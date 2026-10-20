import { readFileSync } from 'node:fs'
import { defineConfig, type DefaultTheme } from 'vitepress'

// Version label for the nav, read from the workspace Cargo.toml at build time.
const cargoToml = readFileSync(new URL('../../Cargo.toml', import.meta.url), 'utf8')
const version = cargoToml.match(/^\[workspace\.package\][^[]*?^version\s*=\s*"([^"]+)"/m)?.[1]
if (!version) throw new Error('No [workspace.package] version in Cargo.toml')

const base = '/'
const repo = 'https://github.com/mavgit1/marcan-dmarc'

const sidebar: DefaultTheme.SidebarItem[] = [
  {
    text: 'Introduction',
    items: [
      { text: 'Getting started', link: '/getting-started' },
      { text: 'How it works', link: '/how-it-works' }
    ]
  },
  {
    text: 'Setup',
    items: [
      { text: 'Deployment', link: '/deployment' },
      { text: 'Mailbox & reporting setup', link: '/mailbox' },
      { text: 'Domains & organizations', link: '/domains' },
      { text: 'Sender names', link: '/sender-names' }
    ]
  },
  {
    text: 'Using Marcan',
    items: [
      { text: 'Reading the dashboard', link: '/dashboard' },
      { text: 'DNS checks', link: '/dns-checks' },
      { text: 'Alerts', link: '/alerts' },
      { text: 'Client reports', link: '/client-reports' },
      { text: 'Exports & API', link: '/exports' }
    ]
  },
  {
    text: 'Operations',
    items: [
      { text: 'Backup & upgrades', link: '/backup' },
      { text: 'Releases', link: '/releases' },
      { text: 'Security model', link: '/security' }
    ]
  },
  {
    text: 'Project',
    items: [
      { text: 'License & pricing', link: '/license' },
      { text: 'Roadmap', link: '/roadmap' },
      { text: 'Development', link: '/development' },
      { text: 'FAQ', link: '/faq' }
    ]
  }
]

export default defineConfig({
  title: 'Marcan',
  description: 'Self-hosted DMARC report processing and mail security monitoring.',
  base,
  cleanUrls: true,
  lastUpdated: false,
  // Local URLs (http://localhost:8080 etc.) in examples are not site links.
  ignoreDeadLinks: [/^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?(\/|$)/],

  head: [
    ['link', { rel: 'icon', type: 'image/svg+xml', href: `${base}favicon.svg` }],
    ['meta', { name: 'theme-color', content: '#155dfc' }]
  ],

  themeConfig: {
    logo: { light: '/logo-light.svg', dark: '/logo-dark.svg', alt: '' },

    nav: [
      { text: 'Guide', link: '/getting-started' },
      { text: `v${version}`, link: `${repo}/tags` }
    ],

    sidebar,

    socialLinks: [{ icon: 'github', link: repo }],

    search: { provider: 'local' },

    footer: {
      message: `Free for your own domains. <a href="${base}license">License &amp; pricing</a> · <a href="https://kaitiaki.tech/impressum">Legal notice</a>`
    }
  }
})
