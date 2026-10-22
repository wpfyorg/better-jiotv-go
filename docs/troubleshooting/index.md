---
title: Troubleshooting JioTV
description: Find and fix common installation, account, playback, guide and server problems.
category: Troubleshooting
keywords: [troubleshooting, errors, playback, login, installation]
---

# Troubleshooting

Start with the symptom that matches what you see. These steps describe the current Rust server and Svelte web app. Provider outages, account entitlements, browser support and network policy can affect results.

Before changing anything, note the app version, install type, browser or IPTV player, whether the problem affects one channel or all channels, and the exact time it happened. Check the server's own terminal or service logs. Do not post OTPs, passwords, cookies, playlist URLs, access keys, tokens, or the private data directory.

## Choose a problem

- [Installation, startup, architecture, permissions and ports](../troubleshooting-installation-startup/index.md)
- [JioTV login, OTP, sessions, tokens and Extras](../troubleshooting-accounts-and-extras/index.md)
- [Channels, loading, buffering and subscriptions](../troubleshooting-channels-and-playback/index.md)
- [HLS, DASH, DRM, Widevine, licenses and browser security](../troubleshooting-protected-playback/index.md)
- [EPG, missing channels, playlists and mode changes](../troubleshooting-epg-and-playlists/index.md)
- [Web access, settings, memory, crashes, Docker and updates](../troubleshooting-server-and-maintenance/index.md)

For setup instructions see [Getting started](../getting-started-in-5-minutes/index.md), [installation guides](../installations/index.md), [configuration](../configuration-reference/index.md), [live TV](../using-live-tv/index.md), and [IPTV playlists](../using-iptv-playlists/index.md).

## Safe first checks

1. Confirm the server is running and use the address and port configured for that installation.
2. Reload the page once and try another channel. This helps separate a single-channel issue from a server-wide issue.
3. Check **Settings** for JioTV, Extras and EPG status. The browser UI's admin login and the JioTV account login are separate.
4. Check the server output around the failure time. Share only a short, redacted excerpt with support.
5. Avoid repeatedly requesting OTPs or changing account, proxy, TLS or firewall settings without evidence.

## Further help

If the relevant article does not resolve it, provide the app version, platform/install method, browser or player and version, the steps to reproduce, whether other channels work, and a redacted error excerpt. Never include credentials or secret-bearing playlist links.
