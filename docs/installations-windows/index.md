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

Close and reopen PowerShell if `jiotv` is not found.

## Guided setup

In a PowerShell window, the installer switches to a guided setup after downloading: build (`full` or `slim`), account (JioTV, JioTV+, or Fastway when the release supports it), the address other devices will use, whether to expose the server through Cloudflare, and then the account sign-in. Use the arrow keys (or `j`/`k`, or the number keys) and press Enter.

The sign-in step asks for the mobile number and one-time password inside the wizard. The OTP is masked, and provider output is captured rather than printed over the screen. For JioTV+ the wizard lists registered connections and asks which one to use; Fastway signs in with a username and password instead. The installer does not store the mobile number or OTP. It saves the selected provider and any named-tunnel token in your Windows user environment. A failed sign-in leaves the binary installed and prints a retry command.

Skip the questions by setting the environment before running the script, or turn the flow off entirely:

```powershell
$env:JIOTV_VARIANT = 'slim'
$env:JIOTV_PROVIDER = 'jiotv'
$env:JIOTV_TUNNEL = 'none'
```

`$env:JIOTV_ONBOARD = 'never'` keeps the plain non-interactive install. Redirected output (CI, scheduled tasks) skips the flow automatically.

The installer configures the account but does not start the server. Start it after setup:

```powershell
jiotv login otp
jiotv admin password
jiotv serve --host 0.0.0.0 --tls
```

Enter the OTP in the terminal. The installer configures the account but does not start the server; run the printed `jiotv serve` command after setup. Open `https://localhost:5443/` on the same PC and accept the self-signed certificate warning. Other devices on your trusted LAN can use `https://<this-PC-ip>:5443/`; find the address with `ipconfig`. IPTV players use the authenticated playlist URL printed by the installer. The `slim` build has no browser interface.

The installer accepts `-Variant full|slim`, `-Version latest|<version>`, `-InstallDir <directory>`, and `-Repo <owner/repository>`. For example, to install a specific release in a custom directory:

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.ps1 -OutFile install-jiotv.ps1
.\install-jiotv.ps1 -Version 1.3.3 -InstallDir "$env:LOCALAPPDATA\JioTV"
```

The installer does not register a Windows service. Keep the terminal running while serving, or use a separately managed service wrapper only if you know how to configure and secure it. Update with `jiotv update`; see [Update and uninstall](../maintenance-update-uninstall/index.md).

## Verify and troubleshoot

Open a new PowerShell window, run `Get-Command jiotv` and `jiotv --help`, then open the selected server address in a browser while the server is running. If Windows cannot find `jiotv`, verify the install folder is on your user `PATH` and reopen PowerShell. If the server cannot bind its port, see [startup errors](../troubleshooting-installation-startup/index.md).

Update with `jiotv update` and restart the server. To uninstall, stop it, remove `jiotv.exe` from the install directory, and remove an unneeded `PATH` entry. See [uninstall and saved data](../maintenance-update-uninstall/index.md).
