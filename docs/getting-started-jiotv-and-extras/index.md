---
title: JioTV, JioTV+ and Fastway: Which Account Do You Need?
description: Understand the three accounts, which network each one needs, and what content they unlock.
category: Getting Started
keywords: [JioTV, JioTV+, Fastway, TV+, provider, subscription, channels, on demand]
---

# JioTV, JioTV+ and Fastway: Which Account Do You Need?

The server can hold three accounts. Your network decides which one you have:

| Account | Needs | Gives you |
| --- | --- | --- |
| **JioTV** | Any network | Live television and programme information |
| **JioTV+** | A JioFiber or AirFiber login (Jio network) | JioTV+ channels, movies and shows |
| **Fastway** | A Fastway subscription, or an ACT connection | Live TV from Fastway |

- **JioTV** is the one to start with. It is the only account that works on any network, so it is enough for a new installation.
- **JioTV+** is the optional second source. It is off until `tv_plus = true` (or `JIOTV_TVPLUS=true`) is set, and it needs its own sign-in with the JioFiber or AirFiber number, separate from JioTV.
- **Fastway** is the optional third source. It is only offered when the installed release supports it; enable it with `fastway = true` (or `JIOTV_FASTWAY=true`) and connect with its own Fastway username and password. See [Connect Fastway Live TV](../account-fastway/index.md).

The installers list only the accounts the downloaded release supports and print this same hint above the account menu. The older name **Extras** may still appear in older guides and error messages. `jiotv extras login` remains a command alias; `jiotv tv+ login` is the current spelling.

## Check which source is active

1. Open **Settings** in the web interface.
2. Find the **JioTV** card. If it says **Not logged in**, use **Send OTP** to connect.
3. If the server has JioTV+ enabled, the **JioTV+** card shows whether that account is connected. A connected account adds content and a small active icon in the header.
4. Open **Channels**. Use **JioTV+ only** if you want to inspect the optional source. If JioTV+ is disconnected, its content might be unavailable.

![Real Settings screen showing a disconnected JioTV account. JioTV+ is disabled in this example.](../images/settings-no-account.png)

The screenshot is from a clean installation with no account connected. Your screen may include more account controls when JioTV+ or Fastway is enabled. The existence of a channel tile never guarantees permission to play it.

## Understand labels and restrictions

**Subscription required** means you may need a plan or additional rights. **Unavailable** can also mean your account is disconnected, the provider rejected playback, the programme is region restricted, or the browser does not support its DRM system. JioTV cannot unlock subscriptions or bypass DRM.

If you need more channels, check the subscription with the provider before changing server settings. See [Connect a JioTV account](../account-connect-jiotv/index.md), [Connect JioTV+](../account-extras/index.md) and [Why a channel will not play](../troubleshooting-channels-and-playback/index.md).
