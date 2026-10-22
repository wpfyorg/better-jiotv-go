---
title: Install on macOS
description: Install and run the JioTV server on Intel or Apple silicon Macs.
category: Installation
keywords: [macOS, Mac, Apple silicon, Intel, install, terminal]
---

# Install on macOS

The installer supports Intel (`x86_64`) and Apple silicon (`arm64`) Macs. It downloads the matching release and checks its SHA-256 checksum.

## Install

Open Terminal and run:

```sh
curl -fsSL https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.sh | sh
```

Without elevated privileges, the binary goes in `$HOME/.local/bin`. Add that directory to your shell's `PATH` if needed. For example, with zsh:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
```

## First run

```sh
jiotv login otp
jiotv admin password
jiotv serve --host 127.0.0.1 --tls
```

Open `https://localhost:5443/` and accept the self-signed certificate warning. To access the server from other devices on your home network, bind to all interfaces with `--host 0.0.0.0` and use your Mac's LAN address. Keep access limited to a trusted network. The `slim` build has no browser interface.

For boot-time service setup, see [Background service](../maintenance-background-service/index.md). For updating and removing the binary, see [Update and uninstall](../maintenance-update-uninstall/index.md).

## Verify and troubleshoot

Run `command -v jiotv` and `jiotv --help` in Terminal. Confirm the browser opens the address selected when starting `serve`. If `jiotv` is missing, confirm your install location is in `PATH`; restart Terminal after updating the shell profile. If a port is occupied, follow [startup fixes](../troubleshooting-installation-startup/index.md).

Update with `jiotv update`, restart JioTV, and check the interface. For removal, stop JioTV and delete the binary from your install directory. Keep your data directory unless you want to erase your saved account state; see [uninstall](../maintenance-update-uninstall/index.md).
