---
title: Requirements and Supported Devices
description: Choose a supported JioTV server and browser before installing.
category: Getting Started
keywords: [requirements, CPU architecture, OpenWrt, Android, Raspberry Pi, browsers, DRM]
---

# Requirements and Supported Devices

You need a device that can run the Rust `jiotv` binary, network access to the content service, and a modern web browser or compatible IPTV application. Watching account-restricted streams requires an eligible JioTV or Extras account.

## Where you can run the server

The repository supplies installers/build or packaging support for Linux, macOS, Windows, OpenWrt, Raspberry Pi and other Linux single-board computers, and Android through **Termux**. It also supplies a Docker image. Exact CPU architecture support varies by release; check the [platform-specific instructions](../installations/index.md) before downloading.

The **full** build serves the browser UI. The **slim** build omits browser assets and is intended for IPTV clients. Extras also requires a build with the `extras` feature; the default Cargo feature set does not include Extras.

## Where you can watch

- **Browser:** open the host and port printed by the server, typically `http://localhost:5001/` on the same machine.
- **Other computers/phones:** you must explicitly bind the server to a network interface and use its reachable address. Avoid exposing it to the public internet.
- **IPTV app:** paste the playlist URL copied from **Settings → IPTV playlist**, and optionally the EPG URL.

For some protected playback, the browser needs a secure origin (`https://`), supported encrypted-media APIs, and a licensed DRM module. A supported browser alone is not sufficient: the content may still be blocked by subscription or provider restrictions.

## Before you begin

1. Confirm your device architecture and download the matching installer or binary.
2. Keep a writable directory for JioTV's state and tokens.
3. Know which device will run the server and which device will open the interface.
4. Have access to your own mobile number and OTP to connect your JioTV account.

Read [What is JioTV?](../getting-started-what-is-jiotv/index.md) or [install for your platform](../installations/index.md).
