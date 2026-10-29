---
title: Install on Android with Termux
description: Run JioTV as a server in Termux on supported Android devices.
category: Installation
keywords: [Android, Termux, phone, tablet, aarch64, ARMv7]
---

# Install on Android with Termux

Install Termux from a trusted distribution source and open its terminal. The JioTV installer detects Termux and chooses a supported Android target. Supported targets are aarch64, ARMv7, and x86_64.

```sh
pkg update
pkg install curl
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

The binary is installed in Termux's `$PREFIX/bin`. The installer also runs the guided setup in the Termux terminal: build (`full` or `slim`), account (JioTV, JioTV+, or Fastway when the release supports it), the address other devices will use, an optional Cloudflare tunnel, and the account sign-in. Arrow keys select and Enter confirms.

To start and sign in later instead, or if you skipped the questions with `JIOTV_ONBOARD=never`:

```sh
jiotv login otp
jiotv admin password
jiotv serve --host 0.0.0.0 --tls
```

Keep Termux open while the foreground server runs. On the phone, use `https://localhost:5443/`; other devices on the same trusted network can use the phone's LAN address. Android may restrict background processes or network access when the app is idle. Termux autostart saves the current `JIOTV_*` settings, including a named-tunnel token, in a private environment file, then adds a command to `bash.bashrc` to start JioTV in the background when a shell opens:

```sh
jiotv autostart
```

If you configure a named tunnel when setting up autostart manually, include its serve flags: `jiotv autostart --args "--tls --tunnel"`. Autostart saves the current `JIOTV_*` values privately and refreshes its startup command when you run it again with different flags.

Remove that startup entry with `jiotv autostart remove`. Update with `jiotv update`. See [Update and uninstall](../maintenance-update-uninstall/index.md) and [data, backups and logs](../configuration-storage-logs/index.md).

## Verify and troubleshoot

Run `command -v jiotv` and `jiotv --help` in Termux, then open the browser page while the server process is running. If installation fails, check that Termux is up to date and that your device has a supported CPU architecture. If playback or the server stops when the screen turns off, investigate Android's background power restrictions; `autostart` alone cannot override them.

Update with `jiotv update`; for removal, run `jiotv autostart remove` if you enabled it, stop the server and delete its executable. Keep or privately back up the data directory. See [uninstall](../maintenance-update-uninstall/index.md) and [startup errors](../troubleshooting-installation-startup/index.md).
