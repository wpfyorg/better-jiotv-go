---
title: Connect JioTV+
description: Enable and connect the optional JioTV+ account source when the server build supports it.
category: Account Setup
keywords: [JioTV+, account, OTP, JioTV+ channels, TV+]
---

# Connect JioTV+

**JioTV+** is an optional second source. It appears in Settings only when JioTV+ is enabled in the server configuration and the build includes JioTV+ support. It may use a registered mobile number and an eligible plan.

1. Open **Settings** and find **JioTV+ channels**.
2. Enter the registered 10-digit mobile number and select **Continue**.
3. If asked, choose the matching connection from the list, then select **Send OTP**.
4. Enter the OTP and select **Connect**.
5. Check that the card says **Connected**. The header also shows a JioTV+ indicator while the source is enabled and connected.
6. Return to **Channels**. Use **JioTV+ channels only** to narrow the list if both sources are present.

**Expected result:** channels supplied by the connected source appear when available. A channel can still require a plan or be unavailable for playback.

To disconnect, select **Disconnect** on the card and confirm. **Lock** turns off the JioTV+ source without disconnecting its provider account; it does not sign out the web interface. The server may hide provider logout controls when logout is disabled in its configuration.

If the source is unavailable in Settings, check that this is a JioTV+-enabled server build and that JioTV+ is enabled in [server configuration](../configuration-reference/index.md).
