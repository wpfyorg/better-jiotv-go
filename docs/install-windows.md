# Install on Windows

In PowerShell, install the x64, x86, or ARM64 build for the current user:

```powershell
irm https://raw.githubusercontent.com/wpfyorg/better-jiotv-go/main/scripts/install.ps1 | iex
```

The default location is `%LOCALAPPDATA%\Programs\JioTV`. The installer verifies `SHA256SUMS` and adds that directory to the user `PATH` idempotently. Parameters include `-Variant slim`, `-Version 1.3.1`, `-InstallDir`, `-Repo owner/name`, and `-NoTls` (or `JIOTV_INSTALL_TLS=0`) to omit the HTTPS instructions from the installer output. Open a new terminal if the updated PATH is not available in the current session.

Release binaries are unsigned; Windows SmartScreen may show a warning. No built-in Windows service manager is provided. Start `jiotv serve --host 0.0.0.0 --tls` from a terminal or use a separately configured service wrapper.

Data uses the Windows profile directory when `HOME` is unavailable. Run `jiotv login otp`, then `jiotv admin password`. Update with `jiotv update`; the updater stages the new executable and replaces it after the current process exits. Uninstall by removing `jiotv.exe` and, if desired, the application data directory.

Browse to `https://<host>:5443/` and accept the one-time self-signed certificate warning; browsers need HTTPS for protected playback. IPTV apps should use the `http://<host>:5001/` playlist. Windows Firewall may prompt to allow `jiotv.exe` on private networks. See [HTTPS in Usage](usage.md#https).
