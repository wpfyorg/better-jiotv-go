---
title: Fix JioTV+ Connection
description: Diagnose JioTV+ not connected, restricted channels and incorrect source state.
category: Troubleshooting
keywords: [JioTV+, TV+, JioTV+ channels, connect, entitlement, subscription]
---

# Fix JioTV+ Connection

## Symptoms
**JioTV+** is missing or disconnected in Settings, its channels do not appear, or JioTV+ content is unavailable after login.

## Possible causes
JioTV+ integration is disabled (its default), no separate JioTV+ session is connected, the subscription lacks access, or the provider rejected the session.

## Diagnosis
1. Check **Settings** for the JioTV+ enabled/connected status and the JioTV account status separately.
2. Verify whether the server was started with `JIOTV_TVPLUS=true` or `tv_plus = true` in TOML.
3. In **Channels**, clear filters such as **JioTV+ channels only** and **Show unavailable** to understand which source is visible.
4. Check the exact error and server logs without exposing account credentials.

## Solution
1. Enable JioTV+ in server configuration and restart if the integration is currently off.
2. Connect your own JioTV+ account in **Settings** or use `jiotv tv+ login` on the server.
3. Retry the current source and refresh external playlists after a successful connection.
4. If a channel needs a subscription, use an entitled account or choose other content.

## Verification
Confirm the JioTV+ card shows connected and check a channel your plan permits. Treat JioTV and JioTV+ connections as separate states.

## Further help
Provide the JioTV+ status, source badge, version and redacted error. Never share tokens or login codes. See [connect JioTV+](../account-extras/index.md) and [account troubleshooting](../troubleshooting-accounts-and-extras/index.md).
