/** @type {import('@docusaurus/plugin-content-docs').SidebarsConfig} */
const sidebars = {
  docsSidebar: [
    "introduction",
    "get-started",
    {
      type: "category",
      label: "Installation",
      link: {type: "doc", id: "install"},
      items: [
        "install-linux",
        "install-macos",
        "install-windows",
        "install-android",
        "android-tv",
        "install-docker",
        "install-openwrt",
        "install-sbc",
        "install-homelab",
      ],
    },
    "usage",
    "config",
    "iptv",
    "updating",
    "faq",
    "troubleshooting",
  ],
};

module.exports = sidebars;
