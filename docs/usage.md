# Usage and CLI

The main commands are:

```text
jiotv login otp
jiotv admin password
jiotv serve [--host HOST] [--port PORT] [--tls] [--tls-port PORT] [--tls-cert PEM --tls-key PEM]
jiotv epg generate|delete
jiotv autostart [--args "serve flags"]
jiotv update [--version VERSION]
```

Run `jiotv --help` for the options supported by the installed build. Keep the data directory private; it stores authentication material. Use `jiotv serve` to run in the foreground, or configure the platform's service manager as described in its installation guide.

## HTTPS

Browsers only allow Widevine (EME) and Web Crypto (needed for AES-128 HLS) on HTTPS or `localhost`. Opening the web UI at `http://<LAN-IP>:5001/` therefore shows "Needs a secure connection" for protected channels. Start the server with `--tls` to add an HTTPS listener alongside the plain HTTP one:

```sh
jiotv serve --host 0.0.0.0 --port 5001 --tls --tls-port 5443
```

- **Browser:** open `https://<host>:5443/`. Without your own certificate the server creates a self-signed one at `<data dir>/tls/cert.pem` and `tls/key.pem`, reuses it on later starts, and prints the HTTPS URL and its SHA-256 fingerprint at startup. The browser shows a one-time warning: choose Advanced, then proceed to the site (compare the fingerprint if you want to be sure). Keep the data directory so the same certificate is reused; a new certificate means a new warning.
- **Your own certificate:** pass `--tls-cert <cert.pem> --tls-key <key.pem>` (both are required together).
- **IPTV apps:** keep using the `http://<host>:5001/` playlist and guide URLs. TiviMate, Kodi and similar apps often reject self-signed certificates.
- **Tunnel:** `--tunnel` already serves the UI over HTTPS through Cloudflare, so no certificate setup is needed there.

Service installs created by the project's installers and packages enable `--tls` by default. See [Troubleshooting](troubleshooting.md) if the browser still reports "Needs a secure connection".
