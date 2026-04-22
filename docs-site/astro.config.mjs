import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// Base path matches the GH Pages project URL (unhappychoice.github.io/splashboard/).
// Internal links in content are relative, so they're unaffected; only the public URL uses this.
export default defineConfig({
  site: 'https://unhappychoice.github.io',
  base: '/splashboard',
  integrations: [
    starlight({
      title: 'splashboard',
      description: 'Customizable terminal splash — fetcher × renderer reference.',
      customCss: ['./src/styles/snapshot.css'],
      social: [
        {
          icon: 'github',
          label: 'GitHub',
          href: 'https://github.com/unhappychoice/splashboard',
        },
      ],
      sidebar: [
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
