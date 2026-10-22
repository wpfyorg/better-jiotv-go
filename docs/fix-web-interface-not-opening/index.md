---
title: Fix Web Interface Not Opening
description: Check server process, listening port, firewall and address when the page cannot load.
category: Troubleshooting
keywords: [web inaccessible, connection refused, port, firewall, localhost, LAN, TLS]
---

# Fix Web Interface Not Opening

## Symptoms
The browser shows **Connection refused**, **Site cannot be reached**, a timeout, or a certificate warning when opening JioTV.

## Possible causes
The server is stopped, the address or port is wrong, the process listens only on localhost, a firewall blocks access, or the browser is using HTTP for an HTTPS-only port.

## Diagnosis
1. On the server, run `jiotv serve` in a terminal if it is not running as a service. Look for startup or port-binding errors.
2. Open `http://localhost:5001/` on that same machine using the default listener.
3. For another LAN device, check whether the server was started with `--host 0.0.0.0` and whether its firewall permits the chosen port.
4. When `--tls` is enabled, use `https://<server-address>:5443/` for the HTTPS listener. Plain HTTP remains on port 5001 by default.

## Solution
1. Start the server and use its configured host and port. For an occupied port, choose an unused one such as `jiotv serve --port 5002`.
2. For trusted LAN access, bind to the intended interface and allow the chosen port in the host firewall.
3. If HTTPS is enabled, use the HTTPS URL. Trust a self-signed certificate only if you control and trust this server.

## Verification
Load **Channels** or the admin sign-in page on the server, then repeat from the target client. A page opening alone does not prove channel playback.

## Further help
Share the URL's scheme/host/port (no access key), browser error, server startup output and network topology. See [network access](../configuration-network-security/index.md) and [server troubleshooting](../troubleshooting-server-and-maintenance/index.md).
