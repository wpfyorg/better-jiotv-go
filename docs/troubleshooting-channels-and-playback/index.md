---
title: Fix channel loading and playback problems
description: Troubleshoot missing channels, loading failures, buffering and account restrictions.
category: Troubleshooting
keywords: [channels, loading, buffering, playback, subscription]
---

# Channel loading and playback

## Channel list is empty, incomplete or keeps loading

### Symptoms

Channels do not appear, the list is empty, or only some categories/languages are shown.

### Possible causes

- The JioTV account is not connected or the channel request failed.
- Search, category, language or Extras-only filters hide results.
- The selected account/source does not provide those channels.
- The provider catalogue is temporarily unavailable.

### Diagnosis

Check **Settings** for JioTV and Extras status. Clear the search field and reset category, language and Extras-only filters in Channels. Reload once, then check server output. Compare the UI list with the account's available content; do not assume every provider channel is available to every account.

### Solution

Reconnect JioTV if disconnected. Reset the filters and choose the intended source. If only a specific source is affected, check its account status and entitlement. Wait and retry later if the upstream catalogue request failed.

### Verification

Confirm a normal channel appears with filters cleared. Test one JioTV channel and one Extras channel only if both accounts are configured.

### Further help

Include filters used, account status labels, whether the issue affects all categories, and a redacted error. See [channel navigation](../using-live-tv/index.md), [account setup](../account-connect-jiotv/index.md), and [Extras](../account-extras/index.md).

## A channel never starts or reports stream unavailable

### Symptoms

The player stays on loading, shows “Stream unavailable from provider,” “Playback unavailable,” or the failure happens only on one channel.

### Possible causes

- The provider returned no usable stream or refused the account.
- The channel is unavailable to the active account or its subscription.
- The browser cannot use a required media feature; see [protected playback](../troubleshooting-protected-playback/index.md).
- A network/proxy interruption prevents the server from reaching the provider.

### Diagnosis

Try another channel in the same source, then retry the failing channel. Check whether it is marked Extras or subscription-required. Compare browser playback with an IPTV client only if that client is already configured. Check the server output for the request time. Do not infer a DRM failure from a generic loading spinner alone.

### Solution

Sign in to the correct account and use content covered by that account. Retry a transient provider failure later. If all channels fail, confirm the server has outbound network access and review proxy configuration with the [network guide](../configuration-network-security/index.md). For browser security or Widevine messages, follow [protected playback](../troubleshooting-protected-playback/index.md).

### Verification

Test a second known-available channel, then retry the original. Confirm whether failure is limited to a channel, source, browser or all clients.

### Further help

Share the visible player message, channel source/type (never the secret URL), browser/player version, time, and redacted server error. See [live TV](../using-live-tv/index.md).

## Playback buffers, freezes or stops

### Symptoms

Video starts but repeatedly buffers, freezes, falls behind, or stops after playing briefly.

### Possible causes

- Wi-Fi/LAN quality or insufficient bandwidth between viewer and server.
- Server CPU, memory, storage or container limits are under pressure.
- Provider/CDN instability or a channel-specific stream issue.
- A proxy or firewall interrupts media segments.

### Diagnosis

Check whether the issue affects one device or all viewers, one channel or many, and browser UI or IPTV. Try the server from a nearby wired/LAN client if available. Observe host/container resource use during the problem and check logs for repeated request failures. Change one factor at a time.

### Solution

Improve the network path or reduce competing load. Restart the app only if it has stopped responding and you can do so without disrupting other users. If one channel alone buffers, retry later and report it as a channel-specific provider issue. Avoid changing undocumented quality or proxy settings.

### Verification

Watch for several minutes on the affected client and compare with another channel or client. Note whether the buffer recurs and whether host resource use spikes.

### Further help

Provide server platform, client/browser, wired or Wi-Fi path, channel scope, duration and resource observations. See [server maintenance](../troubleshooting-server-and-maintenance/index.md) and [IPTV playlists](../using-iptv-playlists/index.md).

## Player says subscription required or access denied

### Symptoms

The UI identifies a subscription requirement or playback is rejected by the provider.

### Possible causes

- The selected channel/content needs an entitlement absent from the active account.
- JioTV and Extras accounts differ; a channel belongs to the other source.
- Provider access was denied or the provider returned an account-specific refusal.

### Diagnosis

Check the channel's source badge and subscription marker, then check the corresponding account under **Settings**. Try a channel that does not require the same entitlement. A connected account does not guarantee access to all content.

### Solution

Use the account and subscription that include the content, or choose another available channel. For Extras, complete its separate login and verify the plan with the provider. There is no supported entitlement bypass.

### Verification

Try an entitled channel from the same account. If that works while the restricted item does not, the issue is content/account-specific.

### Further help

Provide the exact message and source badge without sharing account identifiers or URLs. See [account and Extras troubleshooting](../troubleshooting-accounts-and-extras/index.md).
