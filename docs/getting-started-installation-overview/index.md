---
title: Choose the Right Installation Method
description: Decide between the standard installer, Docker, OpenWrt packages and manual installation.
category: Getting Started
keywords: [install, setup, Linux, Windows, macOS, OpenWrt, Termux, Docker, full, slim]
---

# Choose the Right Installation Method

JioTV is a **server**. One device runs it, and a browser or IPTV application opens its interface or playlist over the network. For example, you can run the server on a Raspberry Pi and watch from a laptop on the same home network.

Choose your server device first:

| Your device | Recommended guide |
| --- | --- |
| Linux desktop or server | [Linux](../installations-linux/index.md) |
| Mac desktop or laptop | [macOS](../installations-macos/index.md) |
| Windows PC | [Windows](../installations-windows/index.md) |
| OpenWrt router | [OpenWrt](../installations-openwrt/index.md) |
| Raspberry Pi or Linux mini-computer | [Raspberry Pi and SBC](../installations-sbc/index.md) |
| Android with a terminal app | [Termux](../installations-termux/index.md) |
| Docker host | [Docker and Compose](../installations-docker/index.md) |
| Existing downloaded executable | [Manual binary](../installations-build-from-source/index.md) |
| Developer with a Rust toolchain | [Build from source](../installations-build-from-source/index.md) |

The **full** build serves the web interface. A **slim** build has no web interface and is for IPTV clients. Extras is optional and depends on how the binary was built. Select the full build if this is your first installation.

After installing, use [Getting Started in 5 Minutes](../getting-started-in-5-minutes/index.md). If your browser runs on another device, read [Access JioTV on your home network](../configuration-network-security/index.md).
