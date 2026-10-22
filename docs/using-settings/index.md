---
title: Settings and Account Controls
description: Check account status, manage passwords and playlist access, and find server configuration guidance.
category: Using JioTV
keywords: [settings, admin password, playlist, EPG, access key, accounts]
---

# Settings and Account Controls

Open **Settings** from the top navigation. The page shows IPTV URLs, account connections and the admin password form.

![Settings on a local server showing playlist URLs and disconnected account cards](../images/settings-no-account.png)

This example has no provider account connected. Your account status and private URLs will differ.

## Accounts

The **JioTV** card shows whether the provider account is logged in. The optional **Extra channels** card appears when Extras is enabled and shows whether it is connected. Use each card's login or disconnect controls to manage that provider account. See [JioTV account sign-in](../account-connect-jiotv/index.md) and [Extra channels](../account-extras/index.md).

You can [switch JioTV accounts](../account-switch-jiotv/index.md) by logging out and signing in with another number; an administrator can hide provider logout controls with a server configuration option.

## Playlist and guide links

Copy the **Playlist** URL into a compatible IPTV app. Copy **EPG** for the programme guide when EPG is enabled. **Make a new key** invalidates the old playlist URL, so update every IPTV app afterward. See [IPTV playlists and XMLTV](../using-iptv-playlists/index.md).

## Change the admin password

1. Enter the current password.
2. Enter a new password of at least eight characters.
3. Select **Change password**.

**Expected result:** Settings confirms the change. Other browser sessions are signed out; the current browser stays signed in. See [Admin Sessions and Sign Out](../account-sessions/index.md).

## Server options

The web page does not expose every server setting. EPG generation, Extras availability, logout behavior, network binding and other runtime options are configured on the server. See [Configuration Reference](../configuration-reference/index.md), [CLI Commands](../configuration-cli/index.md), and [Network access and HTTPS](../configuration-network-security/index.md).
