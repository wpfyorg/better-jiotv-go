---
title: Use IPTV Playlists and XMLTV Guide
description: Copy the private M3U playlist and optional XMLTV guide URL into a compatible IPTV player.
category: Using JioTV
keywords: [IPTV, M3U, XMLTV, EPG, playlist, external player]
---

# Use IPTV Playlists and XMLTV Guide

JioTV provides an **M3U playlist** for channel streams and an **XMLTV/EPG** URL for programme schedules. The URLs are available in **Settings → IPTV playlist**.

1. Open JioTV's web interface and select **Settings**.
2. Copy the **Playlist** URL and add it as a network playlist in your IPTV app.
3. If the server has EPG enabled, copy the **EPG** URL and add it as the app's XMLTV/guide source.
4. Refresh the playlist and guide in the IPTV app.

**Expected result:** the app imports the channels from the playlist and, when enabled and available, matches schedule data from the XMLTV guide. Provider availability and IPTV app support affect playback and programme matching.

Treat the playlist URL as a password: its `/k/…` access key grants access to the playlist. Keep it private. If it is exposed, use **Make a new key** in Settings, then replace the playlist URL in every IPTV app. Existing playlist URLs stop working after rotation.

The guide file is off unless server configuration enables EPG. See [server configuration](../configuration-reference/index.md) for the `epg` option and [Network access and HTTPS](../configuration-network-security/index.md) before making the server reachable from other devices.
