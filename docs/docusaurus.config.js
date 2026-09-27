/** @type {import('@docusaurus/types').Config} */
const config = {
  title: "JioTV",
  tagline: "JioTV documentation",
  url: "https://wpfyorg.github.io",
  baseUrl: "/better-jiotv-go/",
  organizationName: "wpfyorg",
  projectName: "better-jiotv-go",
  onBrokenLinks: "throw",
  trailingSlash: false,

  presets: [
    [
      "classic",
      {
        docs: {
          path: "content",
          routeBasePath: "/",
          sidebarPath: require.resolve("./sidebars.js"),
          editUrl:
            "https://github.com/wpfyorg/better-jiotv-go/edit/rust/docs/content/",
        },
        blog: false,
      },
    ],
  ],

  plugins: [
    [
      "@easyops-cn/docusaurus-search-local",
      {
        hashed: true,
        indexBlog: false,
        indexDocs: true,
        indexPages: false,
        language: ["en"],
        docsRouteBasePath: "/",
        docsDir: "content",
      },
    ],
  ],

  themeConfig: {
    colorMode: {
      defaultMode: "dark",
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: "JioTV",
      items: [
        {to: "/get-started", label: "Get started", position: "left"},
        {
          href: "https://github.com/wpfyorg/better-jiotv-go",
          label: "GitHub",
          position: "right",
        },
      ],
    },
    footer: {
      style: "dark",
      links: [
        {
          title: "Docs",
          items: [
            {label: "Get started", to: "/get-started"},
            {label: "Configuration", to: "/config"},
            {label: "Troubleshooting", to: "/troubleshooting"},
          ],
        },
        {
          title: "Project",
          items: [
            {
              label: "GitHub",
              href: "https://github.com/wpfyorg/better-jiotv-go",
            },
          ],
        },
      ],
      copyright: `Copyright © ${new Date().getFullYear()} JioTV contributors`,
    },
  },
};

module.exports = config;
