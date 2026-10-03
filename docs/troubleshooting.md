# Troubleshooting

## Installer reports an unsupported platform

Check `uname -s` and `uname -m`. The project publishes only the architectures listed in [Get started](get-started.md). Android Termux must use an Android target; UserLAnd uses a Linux target.

## Checksum verification fails

Delete the partial download and retry. The installer refuses to install an asset that is absent from `SHA256SUMS` or whose hash differs. Check that the requested release exists and that `JIOTV_REPO` points to the correct repository.

## The server starts but is unreachable on the LAN

Bind to `0.0.0.0`, permit ports 5001 (HTTP) and 5443 (HTTPS) through the host firewall, and use the host's LAN address. For OpenWrt, use a LAN-only firewall rule. Do not expose the service on WAN.

## Login state is missing after restart

Check that `JIOTV_PATH_PREFIX` points to the same persistent directory for interactive login and the service. Container and router deployments must persist their data location.

## Playback or client-specific problems

Confirm the account is logged in and the upstream content is available in the official client. Check the application logs and [Configuration](config.md) before changing authentication or proxy settings.

## "Needs a secure connection" or Shaka error 4042

The browser blocks Widevine (EME) and Web Crypto on plain HTTP pages that are not `localhost`, so protected or AES-encrypted channels fail to start.

1. Open the UI over HTTPS: `https://<host>:5443/`, or use the `--tunnel` URL.
2. If the server was started without `--tls`, add it (`jiotv serve --tls`); on OpenWrt set `option tls '1'` in `/etc/config/jiotv` and restart the service; in Docker publish port 5443 with `-p 5443:5443`.
3. Accept the one-time self-signed certificate warning (Advanced, then proceed). Compare the SHA-256 fingerprint printed at server startup if you want to verify it.
4. Allow port 5443 through the host or LAN firewall.

See [HTTPS in Usage](usage.md#https) for supplying your own certificate. IPTV apps should keep using the `http://<host>:5001/` playlist.
