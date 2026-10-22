---
title: Getting Started in 5 Minutes
description: Install the server, set an admin password, connect your JioTV account and watch a channel.
category: Getting Started
keywords: [quick start, first run, JioTV account, OTP, watch channel, setup link]
---

# Getting Started in 5 Minutes

This is the shortest path from a new installation to browsing live channels. You need your own JioTV account and a computer or router that can run the server. **Five minutes** is the approximate hands-on setup time; downloads and OTP delivery may take longer.

## 1. Install JioTV

Choose the instructions for your device in [Installation](../installations/index.md). On a supported Linux or macOS machine, the repository's installer is:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

Review downloaded scripts before running them if that suits your security policy. On Windows, OpenWrt, Termux and Docker, use the matching platform article.

## 2. Start the server

In a terminal:

```sh
jiotv serve
```

The default address is **http://localhost:5001/** on the server itself. The process must remain running while you watch. The terminal prints a private **Web setup** link on first launch. Copy that complete link into a browser **on the server**; do not share it. It contains your access key.

For a service on another device, see [access from your network](../configuration-network-security/index.md).

## 3. Choose an admin password

The setup page asks you to choose and repeat an **admin password** with at least eight characters. Select **Set password**. Afterwards, use the same password when the page asks you to **Sign in**.

**Important:** If you opened just `/` before setting a password, the page tells you to use the **Web setup** link from the terminal or run `jiotv admin password` on the server.

![Setup notice at the normal JioTV web address before an admin password is configured](../images/first-launch-setup.png)

The screenshot shows the **setup notice**, not the password-entry form. Use the private **Web setup** link from the terminal to open that form.

## 4. Connect your JioTV account

1. Open **Settings** in the top navigation.
2. In the **JioTV** card, start the mobile-number OTP login.
3. Enter the code that arrives for your account and finish the sign-in.
4. Check that the JioTV card says **Logged in**.

This is separate from the admin password. If **Extra channels** appears, it is an optional source and has its own connection flow.

See [Connect your account](../account-connect-jiotv/index.md) if you need help.

## 5. Watch your first channel

1. Choose **Channels** in the top navigation.
2. Use **Search channels**, **Category**, or **Language** to find a channel.
3. Select a channel tile. The live player opens.
4. Use the player controls for pause/resume, volume, quality or full screen as supported.

![Channels page on a clean server before a TV account is connected](../images/channels-no-account.png)

If there are no channels, return to **Settings** and check connection status. A subscription notice means the content may require a plan; DRM playback may require HTTPS. See [Fix channels that do not play](../troubleshooting-channels-and-playback/index.md).

You can also [use an external IPTV app](../using-iptv-playlists/index.md).
