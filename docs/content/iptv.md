# IPTV, M3U, and XMLTV

The server exposes playlist and guide endpoints for compatible IPTV clients. Start the server, open its web UI to obtain the access-key-protected playlist links, and use those links in a client on the same network. The server generates EPG data when `JIOTV_EPG=true`.

Keep the access key in playlist URLs private. Avoid sharing playlists publicly because they can grant access to the server. For reverse proxies, retain authentication and URL encryption; see [Configuration](config.md).
