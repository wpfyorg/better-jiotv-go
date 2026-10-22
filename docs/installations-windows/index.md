---
title: Install on Windows
description: Install the JioTV server on Windows with PowerShell and configure first launch.
category: Installation
keywords: [Windows, PowerShell, install, Windows 11, Windows 10]
---

# Install on Windows

The PowerShell installer supports x64, x86, and ARM64 Windows. It verifies the matching release binary with the release SHA-256 manifest, installs `jiotv.exe` under your local Programs directory, and adds that directory to your user `PATH`.

Open PowerShell and run:

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.ps1 | iex
```

Close and reopen PowerShell if `jiotv` is not found. Then sign in and start the server:

```powershell
jiotv login otp
jiotv admin password
jiotv serve --host 0.0.0.0 --tls
```

Enter the OTP in the terminal. Open `https://localhost:5443/` on the same PC and accept the self-signed certificate warning. Other devices on your trusted LAN can use `https://<this-PC-ip>:5443/`; find the address with `ipconfig`. IPTV players use the HTTP playlist on port `5001`. The `slim` build has no browser interface.

The installer accepts `-Variant full|slim`, `-Version latest|<version>`, `-InstallDir <directory>`, and `-Repo <owner/repository>`. For example, to install a specific release in a custom directory:

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.ps1 -OutFile install-jiotv.ps1
.\install-jiotv.ps1 -Version 1.3.3 -InstallDir "$env:LOCALAPPDATA\JioTV"
```

The installer does not register a Windows service. Keep the terminal running while serving, or use a separately managed service wrapper only if you know how to configure and secure it. Update with `jiotv update`; see [Update and uninstall](../maintenance-update-uninstall/index.md).

## Verify and troubleshoot

Open a new PowerShell window, run `Get-Command jiotv` and `jiotv --help`, then open the selected server address in a browser while the server is running. If Windows cannot find `jiotv`, verify the install folder is on your user `PATH` and reopen PowerShell. If the server cannot bind its port, see [startup errors](../troubleshooting-installation-startup/index.md).

Update with `jiotv update` and restart the server. To uninstall, stop it, remove `jiotv.exe` from the install directory, and remove an unneeded `PATH` entry. See [uninstall and saved data](../maintenance-update-uninstall/index.md).
