---
title: Fix Video Stuck on Loading
description: Diagnose a live player that never starts, without changing security settings.
category: Troubleshooting
keywords: [loading, spinner, stuck, video, stream unavailable, buffering]
---

# Fix Video Stuck on Loading

## Symptoms
You select a channel but the live player keeps showing a spinner, **Loading** or **Reconnecting**, or eventually reports that the stream is unavailable.

## Possible causes
The provider has no usable stream for that channel, the account is not entitled to it, the browser cannot play the required format, or the server has lost provider/network access. A DRM error can look similar before a specific message appears.

## Diagnosis
1. Wait for a detailed player error, then note its exact wording.
2. Select **All channels** and try a different channel available to your account.
3. In **Settings**, verify the corresponding JioTV or Extra channels account is connected.
4. Compare one browser with another on a secure address (HTTPS or localhost). Look at the server's output for the failure time.

## Solution
1. Reconnect a disconnected account. For a subscription label, choose a channel included in your plan.
2. If the player explicitly reports a secure-connection or Widevine problem, follow [DRM playback errors](../fix-drm-playback-errors/index.md).
3. For a temporary network outage, restore connectivity and select **Try again**. If only one channel fails, retry later.

## Verification
Play a known-available channel for several minutes, then retry the original. Record which one works; one working channel does not prove all channels are available.

## Further help
Provide the visible error message, app/browser version, channel source and redacted server log excerpt. Do not share stream URLs, tokens, or account details. See [all playback symptoms](../troubleshooting-channels-and-playback/index.md).
