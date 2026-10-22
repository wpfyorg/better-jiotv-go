---
title: Fix Extra Channels Connection
description: Diagnose Extras/TV+ not connected, restricted channels and incorrect source state.
category: Troubleshooting
keywords: [Extras, TV+, extra channels, connect, entitlement, subscription]
---

# Fix Extra Channels Connection

## Symptoms
**Extra channels** is missing or disconnected in Settings, Extras channels do not appear, or Extras content is unavailable after login.

## Possible causes
Extras integration is disabled (its default), no separate Extras session is connected, the subscription lacks access, or the provider rejected the session.

## Diagnosis
1. Check **Settings** for the Extras enabled/connected status and the JioTV account status separately.
2. Verify whether the server was started with `JIOTV_EXTRAS=true` or `extras = true` in TOML.
3. In **Channels**, clear filters such as **Extra channels only** and **Show unavailable** to understand which source is visible.
4. Check the exact error and server logs without exposing account credentials.

## Solution
1. Enable Extras in server configuration and restart if the integration is currently off.
2. Connect your own Extras/TV+ account in **Settings** or use `jiotv extras login` on the server.
3. Retry the current source and refresh external playlists after a successful connection.
4. If a channel needs a subscription, use an entitled account or choose other content.

## Verification
Confirm the Extras card shows connected and check a channel your plan permits. Treat JioTV and Extras connections as separate states.

## Further help
Provide the Extras status, source badge, version and redacted error. Never share tokens or login codes. See [connect Extras](../account-extras/index.md) and [account troubleshooting](../troubleshooting-accounts-and-extras/index.md).
