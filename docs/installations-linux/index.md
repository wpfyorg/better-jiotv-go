---
title: Install on Linux
description: Install JioTV on supported Linux computers and configure its first run.
category: Installation
keywords: [Linux, install, curl, binary, systemd, Ubuntu, Debian]
---

# Install on Linux

The installer selects a release for x86_64, 64-bit ARM, ARMv7, or 32-bit x86 Linux. It verifies the downloaded binary using `SHA256SUMS` before installing it. ARMv6 and MIPS are not release targets.

## Install

On a machine with `curl` or `wget`, run:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

The default variant is `full`. To choose a variant or version, download the script first, then run it with the environment setting:

```sh
curl -fsSLO https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh
JIOTV_VARIANT=slim JIOTV_VERSION=1.3.3 sh install.sh
```

The installer uses `/usr/local/bin` when run as root and writable; otherwise it uses `$HOME/.local/bin`. Set `JIOTV_INSTALL_DIR` to choose another directory. If the command is not found afterward, add the install directory to `PATH` and open a new terminal.

## Start and sign in

```sh
jiotv login otp
jiotv admin password
jiotv serve --host 0.0.0.0 --tls
```

Enter the one-time password when prompted. The admin password protects the web interface; the JioTV OTP signs the server into your provider account. On the same machine, open `https://localhost:5443/` and accept the self-signed certificate warning. From another trusted LAN device, use `https://<server-ip>:5443/`. IPTV applications use the HTTP playlist on port `5001`. The `slim` build has no browser interface.

To start at boot on a systemd Linux machine, see [Background service](../maintenance-background-service/index.md). To update or remove JioTV, see [Update and uninstall](../maintenance-update-uninstall/index.md).

## Installer options

`JIOTV_VARIANT` accepts `full` or `slim`; `JIOTV_VERSION` accepts `latest` or a release version; `JIOTV_REPO` selects the GitHub owner/repository; and `JIOTV_INSTALL_DIR` sets the binary directory. `JIOTV_INSTALL_TLS=0` changes the installer’s printed next-step instructions only. It does not change a running server's TLS setting.

## Verify and troubleshoot

Run `command -v jiotv` and `jiotv --help` to confirm the command is installed. Open the server address in a browser while the process is running. If the shell says the command is missing, check `$HOME/.local/bin` or your chosen install directory and `PATH`. If startup reports the port is busy, select an unused port with `jiotv serve --port 5002`; see [startup fixes](../troubleshooting-installation-startup/index.md).

To update, use `jiotv update` and restart your service. To uninstall, stop the process or service, then remove the installed executable; see [safe removal](../maintenance-update-uninstall/index.md). Preserve the data directory if you may reinstall.
