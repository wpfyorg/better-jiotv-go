---
title: IPTV, EPG and Streaming Settings
description: Choose guide generation, media proxy and DRM settings safely.
category: Configuration
keywords: [EPG, XMLTV, M3U, proxy, HLS, DASH, DRM, secureurl]
---

# IPTV, EPG and Streaming Settings

Most viewers can leave these settings alone. This reference is for setting up an IPTV app or troubleshooting stream delivery. The values below come from the current Rust configuration schema.

## Programme guide (EPG)

By default `epg = false`, so the XMLTV guide is not generated automatically. To enable it, create or edit a TOML configuration file:

```toml
epg = true
```

You can also start JioTV with the environment variable `JIOTV_EPG=true`. Restart the server after changing the setting. The EPG endpoint is `/epg.xml.gz`; in **Settings**, copy the **EPG** URL shown for your installation. The command `jiotv epg generate` creates a guide, and `jiotv epg delete` removes the generated cache. The programme schedule depends on account access and upstream data.

## M3U playlist and access key

The full web interface shows playlist and EPG links in **Settings**. A protected installation uses an access key in the playlist link. Do not replace the full link with a guessed `/playlist.m3u` URL: its behavior depends on your authentication setup. Use **Copy** for the link supplied by your server.

![Settings shows the M3U and EPG locations and warns that the playlist must stay private.](../images/settings-no-account.png)

If you choose **Make a new key**, clients with old links stop working. Copy the new URL to every IPTV app. The key belongs only to your local server; it does not change your provider subscription.

## Playback options

| Setting | Default | When to change |
| --- | --- | --- |
| `drm` / `JIOTV_DRM` | `true` | Keep enabled for supported protected DASH playback. It does not provide Widevine or a subscription. |
| `disable_ts_handler` / `JIOTV_DISABLE_TS_HANDLER` | `false` | Advanced HLS testing when troubleshooting a transport/segment proxy problem. |
| `disable_url_encryption` / `JIOTV_DISABLE_URL_ENCRYPTION` | `false` | Leave off. Enabling this weakens protection of upstream URL parameters. |
| `proxy` / `JIOTV_PROXY` | empty | Specify an outbound proxy URL only if your network needs one. |

Protected browser playback may need HTTPS and a working Widevine or other supported DRM environment. See [Network and HTTPS](../configuration-network-security/index.md) and [DRM playback errors](../troubleshooting-protected-playback/index.md). For a complete list of settings, see the [configuration reference](../configuration-reference/index.md).
