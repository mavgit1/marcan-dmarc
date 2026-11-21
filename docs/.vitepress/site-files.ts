// What the docs site says about itself to search engines, link previews and language models:
// the canonical link and preview tags of every page, llms.txt and llms-full.txt, and the build
// check that every page has what these need. config.ts wires them in; the sitemap is
// VitePress's own (`sitemap` there). The rules for writing descriptions and the llms.txt
// header are in private/DOCS-SITE.md.
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import type { DefaultTheme, HeadConfig, PageData, SiteConfig, TransformContext } from 'vitepress'

/** The site's address, without a trailing slash: every absolute URL is built from it. */
export const hostname = 'https://marcan.kaitiaki.tech'

/** The social preview image, in docs/public; made by scripts/screenshots/og-image.mjs. */
const ogImage = 'og.png'

/** A page's description: what the build demands of its `description` front matter. */
const descriptionLength = { min: 70, max: 160 }

/**
 * Pages deliberately outside the sidebar, by source path, and what they are:
 * - `home`: the start page, described by the llms.txt header itself;
 * - `optional`: listed under "Optional" in llms.txt, left out of llms-full.txt. The frozen license
 *   texts: their files stay byte for byte as released, so their description is given here;
 * - `moved`: only a pointer to the page named; noindex, canonical to that page, and left out of
 *   the sitemap and the llms files. It needs no description.
 * Every other page must be in the sidebar, so that it reaches llms.txt.
 */
export const outsideSidebar: Record<string, { kind: 'home' } | { kind: 'optional'; description: string } | { kind: 'moved'; to: string }> = {
  'index.md': { kind: 'home' },
  'licenses/bcl-1.0.md': {
    kind: 'optional',
    description:
      "The full text of the Beneficiary Copyleft License 1.0, Marcan's license, which adds a duty to tell clients to the GNU AGPLv3 under Swiss law, kept unchanged."
  },
  'licenses/operator-v1.md': {
    kind: 'optional',
    description:
      'The terms of the Marcan Operator License, Version 1, the paid exemption from telling clients, with its scope, tiers and fees, verification, term and liability.'
  },
  'mailbox.md': { kind: 'moved', to: '/accounts' }
}

/** The page's path on the site, clean: `/` for the home page, `/accounts` for accounts.md. */
function pagePath(relativePath: string): string {
  return '/' + relativePath.replace(/(^|\/)index\.md$/, '$1').replace(/\.md$/, '')
}

const absolute = (path: string) => hostname + path

/** The canonical link and preview tags of one page (`transformHead`). */
export function pageHead({ page, pageData, siteData }: TransformContext): HeadConfig[] {
  if (page === '404.md') return []
  const path = pagePath(pageData.relativePath)
  const special = outsideSidebar[pageData.relativePath]
  if (special?.kind === 'moved') {
    return [
      ['meta', { name: 'robots', content: 'noindex' }],
      ['link', { rel: 'canonical', href: absolute(special.to) }]
    ]
  }
  const title = special?.kind === 'home' ? `${siteData.title}: ${pageData.frontmatter.hero?.text ?? siteData.title}` : pageData.title
  const description: string = pageData.frontmatter.description ?? siteData.description
  const tags: [string, string][] = [
    ['og:type', 'website'],
    ['og:site_name', siteData.title],
    ['og:title', title],
    ['og:description', description],
    ['og:url', absolute(path)],
    ['og:image', absolute(`/${ogImage}`)],
    ['og:image:width', '1200'],
    ['og:image:height', '630'],
    ['og:image:alt', 'The Marcan logo with the line Self-hosted DMARC monitoring']
  ]
  return [
    ['link', { rel: 'canonical', href: absolute(path) }],
    ...tags.map(([property, content]): HeadConfig => ['meta', { property, content }]),
    ['meta', { name: 'twitter:card', content: 'summary_large_image' }]
  ]
}

/** Sitemap entries without the `moved` pointers (`sitemap.transformItems`). */
export function sitemapItems<T extends { url: string }>(items: T[]): T[] {
  const moved = Object.entries(outsideSidebar)
    .filter(([, page]) => page.kind === 'moved')
    .map(([file]) => pagePath(file).slice(1))
  return items.filter((item) => !moved.includes(item.url))
}

/** Every page's data as VitePress parsed it, by source path, collected in `transformPageData`. */
const pages = new Map<string, PageData>()

export function collectPage(pageData: PageData): void {
  // A description given in `outsideSidebar` counts as the page's own, for its meta tag too.
  const special = outsideSidebar[pageData.relativePath]
  if (special?.kind === 'optional') {
    pageData.frontmatter.description = special.description
    pageData.description = special.description
  }
  pages.set(pageData.relativePath, pageData)
}

type Section = { title: string; links: string[] }

/** The sidebar's groups and their pages in order, nested items flattened after their parent. */
function sidebarSections(sidebar: DefaultTheme.SidebarItem[]): Section[] {
  const links = (items: DefaultTheme.SidebarItem[] = []): string[] =>
    items.flatMap((item) => [...(item.link ? [item.link] : []), ...links(item.items)])
  return sidebar.map((group) => ({ title: group.text ?? '', links: links(group.items) }))
}

/** The source path of a sidebar link: `/accounts` → `accounts.md`, `/` → `index.md`. */
function sourceOf(link: string): string {
  const path = link.replace(/[#?].*$/, '').replace(/^\//, '')
  return path === '' || path.endsWith('/') ? `${path}index.md` : `${path}.md`
}

/**
 * The build check: every page has a description of the right length and is in the sidebar or in
 * `outsideSidebar`, every sidebar link is a page, and the preview image exists at its size.
 * Throws with every problem at once, each naming its file.
 */
function check(siteConfig: SiteConfig, sections: Section[]): void {
  const problems: string[] = []
  const inSidebar = new Set(sections.flatMap((s) => s.links.map(sourceOf)))
  for (const file of siteConfig.pages) {
    const special = outsideSidebar[file]
    if (!inSidebar.has(file) && !special) {
      problems.push(`docs/${file}: not in the sidebar (config.ts), so it would miss llms.txt. Add it there, or to outsideSidebar in site-files.ts if it is deliberately outside.`)
    }
    if (special?.kind === 'moved') continue
    const data = pages.get(file)
    if (!data) {
      problems.push(`docs/${file}: VitePress gave no page data for it.`)
      continue
    }
    const description = data.frontmatter.description
    if (typeof description !== 'string' || description.trim() === '') {
      problems.push(`docs/${file}: no \`description\` in its front matter (one sentence, ${descriptionLength.min} to ${descriptionLength.max} characters).`)
    } else if (description.length < descriptionLength.min || description.length > descriptionLength.max) {
      problems.push(`docs/${file}: its \`description\` has ${description.length} characters; it needs ${descriptionLength.min} to ${descriptionLength.max}.`)
    }
  }
  for (const file of inSidebar) {
    if (!siteConfig.pages.includes(file)) problems.push(`config.ts: the sidebar links to ${file}, which is no page.`)
  }
  for (const file of Object.keys(outsideSidebar)) {
    if (!siteConfig.pages.includes(file)) problems.push(`site-files.ts: outsideSidebar names docs/${file}, which is no page.`)
    if (inSidebar.has(file)) problems.push(`site-files.ts: outsideSidebar names docs/${file}, which is in the sidebar too.`)
  }
  const image = join(siteConfig.srcDir, 'public', ogImage)
  if (!existsSync(image)) {
    problems.push(`docs/public/${ogImage} is missing: make it with scripts/screenshots/og-image.mjs.`)
  } else {
    const png = readFileSync(image)
    const [width, height] = [png.readUInt32BE(16), png.readUInt32BE(20)]
    if (width !== 1200 || height !== 630) problems.push(`docs/public/${ogImage} is ${width}x${height}; it must be 1200x630.`)
  }
  if (problems.length > 0) {
    throw new Error(`The docs site is missing what search engines and llms.txt need:\n- ${problems.join('\n- ')}`)
  }
}

/**
 * The header of llms.txt and llms-full.txt (llmstxt.org): facts a model can match against a
 * question. It goes stale silently: re-read it after a change to features, requirements, prices
 * or the license (private/DOCS-SITE.md).
 */
const header = `# Marcan

> Marcan is self-hosted DMARC monitoring software. It imports DMARC aggregate reports from mailboxes (IMAP, or Microsoft 365 over Microsoft Graph), checks the domains' mail DNS records live, and shows the results in a web dashboard. It is one binary with the UI built in, published as a Docker image, and keeps everything in one SQLite file.

What it does:

- Reads aggregate (rua) reports from any number of mail accounts: any IMAP server over TLS or STARTTLS, Gmail and Google Workspace with an app password, Microsoft 365 through an Entra app registration with a certificate. It only reads: it never moves, flags or deletes mail.
- Decides each record's DMARC result itself from the raw SPF and DKIM results, and sorts mail into four outcomes: DMARC pass, blocked by policy, forwarded, delivered failures.
- Checks DMARC, SPF (with its lookup limits), DKIM, MTA-STS, TLS-RPT and BIMI records live, grades them with a DNS score from 0 to 100, and has a DNS inspector for any other domain.
- Says per domain when it is ready to move from p=none to p=quarantine to p=reject, from the last 30 days of reports, and which record to publish.
- Emails alerts when something changes (DNS records, policy, health, reports stopping, a new sender failing DMARC), with an optional daily or weekly digest. Experimental.
- Sends each organization (client) a monthly email report about its own domains, with the operator's name and logo. Experimental.
- Exports domains, senders, authentication issues and DNS findings as CSV. The JSON API is internal and not stable in 0.x.

What is particular to it:

- Little to run: one container and one SQLite file, with no database server, queue or worker beside it; the only other piece is the reverse proxy. A guided setup tests each mailbox login, checks the DNS records live and shows the exact values to publish.
- It says what to do, not only what happened: a readiness verdict per domain with the record to publish next, and a list of the domains that need attention.
- Client domains stay apart: every domain belongs to exactly one organization, every page, total and export can be narrowed to one organization or domain, and a client's report covers its own domains only. (This separates the data shown, not who may see it: whoever the reverse proxy lets in sees every organization.)
- Narrow access to mail: mailboxes are only read, and a Microsoft 365 app is given the one report mailbox, not the tenant's mail.
- No paid tier of features: the Operator License changes the notice to clients, nothing else.

For: people and organizations monitoring their own domains, and MSPs or agencies looking after client domains, grouped into one organization per client.

Needs: a mailbox that receives the reports (each domain's DMARC record names its address in rua=), access to the domains' DNS, a server with Docker (image ghcr.io/mavgit1/marcan, linux/amd64 and linux/arm64), and a reverse proxy in front for HTTPS and the login: Marcan has no user accounts of its own. The docs have Docker Compose stacks for Caddy with basic auth, Authelia and OAuth2-Proxy.

Not: a SaaS with a sign-up (you run it yourself), a mail server (it sends only through one of your own mail accounts), or a DNS host (it reads DNS and never changes it). It does not import forensic/failure (ruf) reports or TLS-RPT reports, and does not validate BIMI mark certificates (VMC).

License and price: free for your own domains under the Beneficiary Copyleft License 1.0 (the AGPLv3 terms plus a duty to tell clients the software is freely available, under Swiss law; not OSI-approved). Whoever runs it for clients tells them, or buys an Operator License instead: Operator 60 (up to 60 client domains, USD 20 a month or 200 a year) or Operator Unlimited (USD 50 a month or 500 a year), each with a 30-day trial. Every feature is in the free software.

Start: [Getting started](${hostname}/getting-started). A local trial is one \`docker run\` command: [Local trial](${hostname}/deployment#local-trial).
`

/** A page's Markdown made readable without VitePress: for llms-full.txt. */
function plainMarkdown(srcDir: string, file: string): string {
  const source = readFileSync(join(srcDir, file), 'utf8')
  const lines = source.replace(/^---\n[\s\S]*?\n---\n/, '').replace(/<!--[\s\S]*?-->\n?/g, '').split('\n')
  const out: string[] = []
  let fence: string | null = null
  for (const line of lines) {
    const fenceMark = line.match(/^\s*(`{3,}|~{3,})/)?.[1]
    if (fence) {
      out.push(line)
      if (fenceMark && fenceMark[0] === fence[0] && fenceMark.length >= fence.length) fence = null
      continue
    }
    if (fenceMark) {
      fence = fenceMark
      out.push(line)
      continue
    }
    // A code snippet imported from the repository: `<<< ../deploy/caddy/Caddyfile{txt} [Caddyfile]`.
    const snippet = line.match(/^<<<\s+(\S+?)(?:\{(\w+)\})?(?:\s+\[([^\]]+)\])?\s*$/)
    if (snippet) {
      const [, path, lang, label] = snippet
      const target = resolve(srcDir, dirname(file), path)
      const language = lang ?? target.match(/\.(\w+)$/)?.[1] ?? ''
      out.push(`${label ?? path} (\`${path.replace(/^(\.\.\/)+/, '')}\` in the repository):`, '', '```' + language, readFileSync(target, 'utf8').trimEnd(), '```', '')
      continue
    }
    // ::: containers: the opening line keeps its title, the markers go.
    const container = line.match(/^:::+\s*(\S+)?\s*(.*)$/)
    if (container) {
      const [, kind, title] = container
      const name = kind ? kind[0].toUpperCase() + kind.slice(1) : ''
      if (kind && kind !== 'code-group') out.push(title ? `**${name}: ${title}**` : `**${name}:**`)
      continue
    }
    out.push(
      line
        .replace(/<Screenshot\s[^>]*?alt="([^"]*)"[^>]*\/>/g, '(Screenshot: $1)')
        .replace(/<a\s[^>]*?href="([^"]*)"[^>]*>(.*?)<\/a>/g, '[$2]($1)')
        .replace(/<\/?(div|p|span|figure)\b[^>]*>/g, '')
        .replace(/^(#+ .*?)\s*\{#[^}]+\}\s*$/, '$1')
        .replace(/\s*\{\.[\w-]+\}\s*$/, '')
        .replace(/\]\((\.\/|\/)(?!\/)([^)]*)\)/g, (_, prefix: string, target: string) => {
          // Site links become absolute: ./accounts#x and /licenses/bcl-1.0 alike.
          const path = prefix === './' ? join('/', dirname(file), target) : '/' + target
          return `](${hostname}${path.replace(/\.md(?=$|#)/, '')})`
        })
        .trimEnd()
    )
  }
  return out.join('\n').replace(/\n{3,}/g, '\n\n').trim()
}

/** Writes llms.txt and llms-full.txt into the build output, after checking every page (`buildEnd`). */
export function writeLlmsFiles(siteConfig: SiteConfig, sidebar: DefaultTheme.SidebarItem[]): void {
  const sections = sidebarSections(sidebar)
  check(siteConfig, sections)

  const describe = (file: string) => {
    const data = pages.get(file)!
    return `- [${data.title}](${absolute(pagePath(file))}): ${data.frontmatter.description}`
  }
  const optional = Object.entries(outsideSidebar)
    .filter(([, page]) => page.kind === 'optional')
    .map(([file]) => file)
  const index = [
    header,
    ...sections.map((s) => `## ${s.title}\n\n${s.links.map(sourceOf).map(describe).join('\n')}\n`),
    `## Optional\n\n${optional.map(describe).join('\n')}\n`
  ].join('\n')
  writeFileSync(join(siteConfig.outDir, 'llms.txt'), index)

  const full = [
    header,
    ...sections.flatMap((s) =>
      s.links
        .map(sourceOf)
        .map((file) => `Source: ${absolute(pagePath(file))}\n\n${plainMarkdown(siteConfig.srcDir, file)}\n`)
    )
  ].join('\n')
  writeFileSync(join(siteConfig.outDir, 'llms-full.txt'), full)
}
