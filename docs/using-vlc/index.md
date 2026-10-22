---
title: Use JioTV with VLC
description: Open your server's M3U playlist in VLC and understand external player limitations.
category: Using JioTV
keywords: [VLC, IPTV, M3U, external player, network stream, playlist]
---

# Use JioTV with VLC

VLC can read M3U playlists. Whether a particular JioTV stream plays depends on its format, DRM, the connected account and the provider.

## Open the playlist

1. Start the JioTV server and sign in to your JioTV account.
2. Open JioTV **Settings** in your browser. Find **IPTV playlist** and copy the complete **Playlist** URL.
3. Open VLC on a computer on the same trusted network as your server.
4. Choose **Media → Open Network Stream** on Windows/Linux or **File → Open Network** on macOS.
5. Paste the copied URL, then choose **Play** or **Open**. Use VLC's playlist panel to select a channel.

**Expected result:** VLC imports the playlist and lets you choose a channel. Some channels may fail because the player does not support that stream's DRM or the account cannot access the content.

## Troubleshoot VLC

If VLC cannot open the playlist, confirm the JioTV server is running and the address and port are reachable from the VLC device. Copy the URL from Settings again instead of typing it. A rotated access key invalidates old URLs.

If channels appear but a stream fails, try an ordinary, available channel and check the JioTV server log around the failure. Test in the JioTV browser player if available. VLC is not a Widevine playback solution; protected DASH content may remain unavailable.

The M3U address contains a private server access key. Never post it in forums, issue reports, or screenshots. For more help see [IPTV playlist setup](../using-iptv-playlists/index.md), [Network and HTTPS](../configuration-network-security/index.md), and [Playback problems](../troubleshooting-channels-and-playback/index.md).
