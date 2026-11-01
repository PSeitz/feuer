export default {
  title: 'Feuer',
  tagline: 'Cost-aware caching in memory and on disk',
  url: 'https://feuer-rs.github.io',
  baseUrl: '/',
  organizationName: 'feuer-rs',
  projectName: 'feuer-rs.github.io',
  onBrokenLinks: 'throw',
  presets: [
    ['classic', {
      docs: {
        routeBasePath: '/',
        sidebarPath: './sidebars.js',
        editUrl: 'https://github.com/PSeitz/feuer/edit/main/website/',
      },
      blog: false,
    }],
  ],
  themeConfig: {
    navbar: {
      title: 'Feuer',
      logo: {
        alt: 'Feuer flaming horse',
        src: 'https://raw.githubusercontent.com/PSeitz/feuer/main/horse_logo.svg',
      },
      items: [
        { type: 'docSidebar', sidebarId: 'docs', label: 'Guide', position: 'left' },
        { href: 'https://docs.rs/feuer', label: 'API', position: 'right' },
        { href: 'https://github.com/PSeitz/feuer', label: 'GitHub', position: 'right' },
      ],
    },
    prism: {
      additionalLanguages: ['rust', 'toml'],
    },
  },
};
