---
title: Fix HLS, DASH, DRM and browser playback
description: Diagnose secure-context, Widevine, license and protected-stream failures.
category: Troubleshooting
keywords: [HLS, DASH, DRM, Widevine, license, browser, HTTPS]
---

# HLS, DASH, DRM and browser playback

## “Needs a secure connection” or playback fails on a LAN HTTP address

### Symptoms

The web player says it needs a secure connection, or encrypted/protected media fails at `http://<LAN-address>:5001` while the page itself opens.

### Possible causes

Modern browsers restrict encrypted media APIs and some cryptographic media features to secure contexts. `localhost` is treated specially; a LAN IP over plain HTTP is not.

### Diagnosis

Check the exact address in the browser. A plain `http://localhost:5001` test can behave differently from a LAN IP. Check whether TLS was enabled for the server. The Rust CLI adds HTTPS only when started with `--tls`.

### Solution

Enable TLS using `jiotv serve --host 0.0.0.0 --port 5001 --tls` and open `https://<server-lan-address>:5443/`. The generated certificate is self-signed, so accept its warning only when you trust that server, or configure a trusted certificate using both `--tls-cert` and `--tls-key`. Keep IPTV clients on the HTTP playlist listener where needed.

### Verification

Confirm the browser address begins with `https://` and retry the same channel. A local `localhost` test can help isolate secure-context behavior, but does not prove other devices can reach the server.

### Further help

Share browser/version, exact non-secret address scheme/port, and visible player message. See [network and HTTPS](../configuration-network-security/index.md) and [command-line options](../configuration-cli/index.md).

## Widevine or protected content is unavailable

### Symptoms

The player says protected playback is unavailable, or a DASH channel fails with a Widevine/key-system error.

### Possible causes

- The browser or device has no usable Widevine CDM, or its EME implementation cannot create a session.
- The browser context is not secure.
- That channel has no usable stream or the provider refused the account.
- The server has DRM disabled in configuration.

### Diagnosis

First use HTTPS or localhost. Read the UI's specific protected-playback message: the app checks whether EME is available, whether Widevine key-system access succeeds, and whether it can create a MediaKeys session. Try a current browser/device known to support Widevine and compare an ordinary, non-protected channel. Check `drm` / `JIOTV_DRM` in the [configuration reference](../configuration-reference/index.md); its current default is enabled.

### Solution

Use a browser/device with a working Widevine implementation and a secure origin. If the server's DRM setting was deliberately disabled, enable it and restart the server. If only one channel fails, retry later or use another available channel; provider availability cannot be fixed by changing the browser.

### Verification

Confirm the UI no longer reports a missing Widevine/key-system capability and test the affected channel again. Check a second protected channel to distinguish device support from a single stream problem.

### Further help

Report browser/device/version, HTTPS or localhost, exact visible error, and whether another protected channel works. Never provide license request bodies or token-bearing URLs. See [channel playback](../troubleshooting-channels-and-playback/index.md).

## License request fails

### Symptoms

The page loads but a protected stream fails during playback, often with a license-request or HTTP status error.

### Possible causes

- The browser could not reach the server's license proxy or the server could not reach the provider.
- The account lacks access, or its provider session needs refreshing.
- The browser cannot use Widevine in this context.
- Provider license service is temporarily unavailable.

### Diagnosis

Confirm HTTPS and that the error occurs on a protected channel. Try another protected channel and a regular channel. Check **Settings** for the matching account connection and inspect the server output at the failure time. The UI may expose a browser player error code; treat it as a clue, not proof of one specific cause.

### Solution

Reconnect the relevant account if it is signed out, use an entitled channel, and retry later for a provider outage. Correct the HTTPS/browser capability issue if the app identifies one. Do not copy, replay, edit or share license requests, license responses, CDM data, or secret playback URLs.

### Verification

Retry once after the relevant fix and check another protected stream. If regular HLS works but all protected channels fail, report that distinction with redacted server output.

### Further help

Provide the visible error code/message, browser/device, account connection status and time. See [account troubleshooting](../troubleshooting-accounts-and-extras/index.md) and [data and logs](../configuration-storage-logs/index.md).

## HLS channel fails, including encrypted segments

### Symptoms

A channel advertised as HLS does not start, stops after the manifest, or encrypted media fails in the browser.

### Possible causes

- Browser secure-context restriction on a LAN HTTP page.
- Provider/CDN returned an expired or unavailable stream.
- Network, proxy or firewall blocks media requests.
- A channel has multiple stream types and one path is unavailable.

### Diagnosis

Try the channel over HTTPS, then another channel. If IPTV works but browser playback fails, compare the page scheme and browser capability first. If only one channel fails, check again later and inspect server logs for that request. Do not paste or manually open encrypted upstream URLs.

### Solution

Use HTTPS for the browser on LAN devices. Ensure the server has outbound connectivity to its configured provider path. Retry transient or channel-specific failures later. Keep the server's URL protection enabled; do not disable it as a playback workaround.

### Verification

Retry the channel in the browser and, separately, the configured IPTV client. Record which client succeeds and whether playback begins and continues.

### Further help

Include browser/client, secure or plain HTTP, channel scope and redacted logs. See [live TV](../using-live-tv/index.md), [IPTV](../using-iptv-playlists/index.md), and [network and HTTPS](../configuration-network-security/index.md).
