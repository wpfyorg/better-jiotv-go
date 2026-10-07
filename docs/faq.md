# FAQ

## Which variant should I install?

Choose `full` for the built-in web UI. Choose `slim` for headless IPTV-only use.

## Which CPUs are supported?

See the platform table in [Get started](get-started.md). MIPS, MIPS64, ARMv6, and other unlisted targets are not currently release targets.

## Does the app manage Windows or macOS services?

No. Run it in a terminal or configure the platform's service manager separately. Linux systemd, Termux startup, and OpenWrt `procd` are documented where applicable.

## Where is application state stored?

By default it is under `~/.jiotv_go` (or the Windows profile directory if `HOME` is not set). Set `JIOTV_PATH_PREFIX` to choose a persistent path.

## Are the binaries signed?

The macOS and Windows binaries are currently unsigned. The installers verify release checksums.

## Why does the web UI ask for HTTPS?

Browsers only expose Widevine (EME) and Web Crypto on HTTPS or `localhost`. Protected channels and AES-128 HLS therefore fail on plain `http://<LAN-IP>:5001/`. Use `https://<host>:5443/` (self-signed certificate, accept the one-time warning) or the `--tunnel` URL. See [HTTPS in Usage](usage.md#https).

## Should my IPTV app use the HTTPS address?

No. Keep the `http://<host>:5001/` playlist. Many IPTV apps reject self-signed certificates, and they do not need a secure context.
