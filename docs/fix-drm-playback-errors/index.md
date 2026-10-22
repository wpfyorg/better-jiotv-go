---
title: Fix DRM and Widevine Playback Errors
description: Diagnose browser secure-context, Widevine and protected-stream license errors.
category: Troubleshooting
keywords: [DRM, Widevine, Shaka, DASH, license, encrypted media, secure connection]
---

# Fix DRM and Widevine Playback Errors

## Symptoms
A protected channel displays **Protected playback unavailable**, asks for a secure connection, or fails to obtain a media license.

## Possible causes
The page is on LAN HTTP, the browser or device lacks a working Widevine Content Decryption Module, the provider denied the license, the account lacks subscription rights, or the server cannot reach its provider.

## Diagnosis
1. Check the browser address. A plain LAN IP starting with `http://` may prevent encrypted playback, while localhost is treated specially.
2. Open the same JioTV server via configured HTTPS, then retry the channel.
3. Try a non-protected channel and another protected channel, if both are available to your account.
4. Note the player error and browser/device version. Check **Settings** account status and the server's timestamped logs.

## Solution
1. On your trusted LAN, run the server with `jiotv serve --host 0.0.0.0 --tls` or configure the equivalent TLS-enabled service.
2. Browse to `https://<server-address>:5443/`. A generated self-signed certificate must be trusted on that device. For routine shared access, prefer a trusted certificate.
3. Use a browser/device with a working Widevine implementation and an account that can access the channel. A successful API capability check alone does not prove usable DRM playback.
4. If the provider refused the stream or license, retry later or choose permitted content.

## Verification
On the same browser and secure origin, confirm that the protected-player warning disappears and the stream actually plays. Compare with an ordinary channel to isolate the issue.

## Further help
Share the exact displayed error, browser/device version, HTTPS status, and redacted logs. Do **not** share license responses, session tokens, encrypted stream URLs, or try to bypass DRM. For HLS and DASH specifics, read [protected playback troubleshooting](../troubleshooting-protected-playback/index.md).
