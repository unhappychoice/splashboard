import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import starlightLlmsTxt from 'starlight-llms-txt';

export default defineConfig({
  site: 'https://splashboard.unhappychoice.com',
  // Disable smart-quote / em-dash substitution. The llms-*.txt outputs are derived from
  // the rendered markdown, so smartypants would splatter U+2019 / U+2014 into what's
  // supposed to be a plain-text feed — valid UTF-8 but ugly on any client that assumes
  // ASCII, and noise for LLM ingestion since the source itself never had curly quotes.
  markdown: { smartypants: false },
  integrations: [
    starlight({
      title: 'splashboard',
      description: 'Customizable terminal splash — fetcher × renderer reference.',
      customCss: ['./src/styles/theme.css', './src/styles/snapshot.css'],
      plugins: [starlightLlmsTxt()],
      // OG / Twitter preview card. Starlight already emits og:title / og:description /
      // twitter:card, but not the image; add it site-wide so every page shares the same
      // card. Absolute URLs required for social platforms.
      head: [
        {
          tag: 'meta',
          attrs: {
            property: 'og:image',
            content: 'https://splashboard.unhappychoice.com/og.png',
          },
        },
        {
          tag: 'meta',
          attrs: {
            name: 'twitter:image',
            content: 'https://splashboard.unhappychoice.com/og.png',
          },
        },
      ],
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/unhappychoice/splashboard',
        },
      ],
      sidebar: [
        {
          label: 'Guides',
          items: [
            { label: 'Getting started', link: '/guides/getting-started/' },
            {
              label: 'Concepts',
              items: [
                { label: 'Overview', link: '/guides/concepts/' },
                { label: 'Widget', link: '/guides/concepts/widget/' },
                { label: 'Shape', link: '/guides/concepts/shape/' },
                { label: 'Fetcher', link: '/guides/concepts/fetcher/' },
                { label: 'Renderer', link: '/guides/concepts/renderer/' },
              ],
            },
            { label: 'Configuration', link: '/guides/configuration/' },
            { label: 'Presets', link: '/guides/presets/' },
            { label: 'Themes', link: '/guides/themes/' },
            { label: 'ReadStore', link: '/guides/read-store/' },
            { label: 'Trust model', link: '/guides/trust/' },
            { label: 'Cookbook', link: '/guides/cookbook/' },
          ],
        },
        { label: 'Showcases', link: '/showcases/' },
        {
          label: 'Reference',
          items: [
            { label: 'Overview', link: '/reference/matrix/' },
            { label: 'Fetchers', autogenerate: { directory: 'reference/fetchers' } },
            { label: 'Renderers', autogenerate: { directory: 'reference/renderers' } },
          ],
        },
      ],
    }),
  ],
});
