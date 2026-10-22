---
title: Network Access, Ports and HTTPS
description: Make the JioTV web interface reachable and protect streams and account access.
category: Configuration
keywords: [host, port 5001, TLS 5443, public, LAN, authentication, security, DRM]
---

# Network Access, Ports and HTTPS

By default JioTV listens on `localhost:5001` so only the server can connect. A TV or phone on your home network needs an address the TV can reach.

## Open the web interface on your network

1. Find your server's private LAN address (for example, `192.168.1.20`).
2. Start `jiotv serve --host 0.0.0.0 --port 5001` on a trusted network.
3. On another device on the same LAN, visit `http://192.168.1.20:5001/` with **your own server address**.
4. Sign in with your admin password. Ensure any local firewall permits only the devices you intend.

The option `--public` binds to all IPv6 interfaces; its meaning is broader than “allow my TV.” Avoid router port-forwarding for this service.

## HTTPS and protected browser playback

Run `jiotv serve --tls` to add HTTPS on port **5443** while retaining the HTTP IPTV listener on **5001**. Browsers may require a secure origin for DRM/Widevine or encrypted media APIs. The generated certificate is self-signed, so plan for proper trust on each viewing device or provide `--tls-cert` and `--tls-key`. Supplying certificate flags alone, without `--tls`, has no effect.

When you open **Settings → IPTV playlist**, the UI offers plain-HTTP playlist/EPG links when accessed over JioTV's HTTPS listener because some IPTV players do not trust a self-signed certificate.

## Account and playlist security

- Keep `disable_auth = false` and `disable_url_encryption = false`.
- Store your server data directory privately. It contains sessions, login tokens and access keys.
- Never paste an `/k/<key>/...` playlist URL into public forums, screenshots or issue reports.
- **Make a new key** in Settings invalidates old IPTV links on all clients; update those clients afterwards.
- An admin password change signs other browser sessions out.
- Sign out of the web interface when using a shared browser.

For config values, see [Configuration Reference](../configuration-reference/index.md); for lost access, see [troubleshooting](../troubleshooting/index.md).
