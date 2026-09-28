import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  site: 'https://docs.reactor.cloud',
  output: 'static',
  trailingSlash: 'always',
  vite: { plugins: [tailwindcss()] },
  integrations: [
    starlight({
      title: 'Reactor',
      description: 'Auth, data, storage, functions, and sites on one server.',
      favicon: '/favicon.svg',
      lastUpdated: false,
      logo: {
        src: './src/assets/logo.svg',
        alt: 'Reactor',
      },
      customCss: ['./src/styles/custom.css'],
      sidebar: [
        { label: 'Introduction', link: '/' },
        {
          label: 'Start',
          items: [
            { slug: 'start/concepts' },
            { slug: 'start/quickstart' },
            { slug: 'start/projects' },
          ],
        },
        {
          label: 'Product',
          items: [
            { slug: 'product/auth' },
            { slug: 'product/data' },
            { slug: 'product/storage' },
            { slug: 'product/functions' },
            { slug: 'product/sites' },
          ],
        },
        {
          label: 'Operate',
          items: [
            { slug: 'operate/console' },
            { slug: 'operate/cli' },
            { slug: 'operate/configuration' },
            { slug: 'operate/self-hosting' },
            { slug: 'operate/lambda' },
            { slug: 'operate/security' },
          ],
        },
        {
          label: 'Clients',
          items: [
            { slug: 'clients/javascript' },
            { slug: 'clients/swift' },
            { slug: 'clients/kotlin' },
            { slug: 'clients/http' },
          ],
        },
        {
          label: 'Example',
          items: [{ slug: 'examples/todos' }],
        },
      ],
    }),
  ],
});
