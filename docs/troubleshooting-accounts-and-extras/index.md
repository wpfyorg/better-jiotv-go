---
title: Fix JioTV login and Extras problems
description: Diagnose OTP delivery, rejected codes, browser sessions, expired logins and Extras access.
category: Troubleshooting
keywords: [login, OTP, session, token, Extras, account]
---

# JioTV login, sessions and Extras

## OTP is not received or is rejected

### Symptoms

The OTP request fails, no code arrives, or verification reports an invalid code.

### Possible causes

- Number is mistyped or not in the format expected by the provider.
- Delivery is delayed, the code expired, or a newer request invalidated an earlier code.
- The provider rejected the request or verification temporarily failed.
- The Extras account has multiple eligible connections and the wrong one was selected.

### Diagnosis

Check the phone number, the visible app error and server output. The CLI flow is `jiotv login otp`; it prompts in the terminal. The web UI has a separate JioTV OTP flow under **Settings**. Extras has its own connection and OTP flow, also under **Settings**. Confirm which account flow you are using.

### Solution

Request one new OTP, wait for it, and enter that latest code promptly. For Extras, select the intended connection if the interface offers a choice. Do not send an OTP to anyone or include it in diagnostics. If delivery repeatedly fails, wait and contact the provider through its official support route.

### Verification

Reopen **Settings** and confirm JioTV shows as connected. For Extras, confirm it is enabled and connected. Try loading the channel list.

### Further help

Provide which login flow failed, approximate time, platform and redacted error. See [Connect JioTV](../account-connect-jiotv/index.md) and [Extras guide](../account-extras/index.md).

## Web admin password or browser session does not work

### Symptoms

The web UI repeatedly asks for the admin password, rejects it, or logs out after a restart.

### Possible causes

- The web admin password is being confused with the JioTV account login; they are separate.
- The service is reading a different data directory from the one used during setup.
- The password changed, which invalidates existing browser sessions.
- Browser cookies are blocked, cleared or scoped to another host/protocol.

### Diagnosis

Check that you are opening the same host and protocol each time and that the running service uses the expected persistent `JIOTV_PATH_PREFIX`. Compare the service user and data path with the setup command. Do not inspect or share the stored password hash or session secret.

### Solution

Use the admin password set with `jiotv admin password` to enter the web UI. Use JioTV OTP login separately for provider channels. Keep the persistent data directory mounted and consistent across restarts. If you intentionally changed the admin password, sign in again in each browser.

### Verification

Sign in, reload the page, and check **Settings** for provider account state. Restart only after confirming the correct persistent data path is configured.

### Further help

See [first launch](../getting-started-first-launch/index.md), [storage and logs](../configuration-storage-logs/index.md), and [connect JioTV](../account-connect-jiotv/index.md).

## Channels stop working after login or restart

### Symptoms

The account appears signed out, live links fail after a while, or the issue began after changing hosts or restoring data.

### Possible causes

- Provider credentials or session tokens need renewal or re-authentication.
- The server started with a new or different data directory.
- Provider account state changed or an upstream service is temporarily unavailable.
- Cached links from before an account switch are being reused by an external player.

### Diagnosis

Check **Settings** account status and server logs at the time of failure. Try the same channel again from the current UI and compare with another channel. The server refreshes provider credentials as part of its playback requests; there is no supported manual token-copy or token-edit recovery procedure.

### Solution

If Settings shows disconnected, use the normal JioTV OTP login again. Keep the same persistent data directory. Refresh or replace stale playlist entries through the current UI after an account or key change. If only one channel fails, continue with [channel playback troubleshooting](../troubleshooting-channels-and-playback/index.md).

### Verification

Confirm connected status, reload the channel list, then test one channel in the web UI. Check an IPTV player separately if that is where the failure remains.

### Further help

Report whether the failure affects browser playback, IPTV, or both, plus a redacted server error and time. See [IPTV playlists](../using-iptv-playlists/index.md) and [data and logs](../configuration-storage-logs/index.md).

## Extras is missing, disconnected or says a plan is required

### Symptoms

Extras channels are absent, the UI says Extras is not enabled or connected, OTP fails, or playback reports that content is not in the plan.

### Possible causes

- Extras is disabled by configuration or explicitly locked in Settings.
- The separate Extras account has not been connected.
- The account does not have the required entitlement for that item.
- The Extras service or catalogue request failed upstream.

### Diagnosis

In **Settings**, check whether Extras is enabled and connected. If the page offers **Unlock** or **Lock**, note that this UI gate can override the configured default. Then check whether the channel is marked as Extras and whether the message names a plan/entitlement problem. An enabled switch does not itself grant a subscription.

### Solution

Enable Extras in the supported configuration or unlock it in Settings, then complete the Extras OTP flow separately from JioTV login. Choose the intended connection when prompted. If the UI says the account lacks a plan, check the subscription with the provider; do not try to bypass the entitlement.

### Verification

Confirm **Settings** reports Extras connected, refresh Channels, and try an item your account is entitled to. Test a regular JioTV channel to distinguish the two sources.

### Further help

Provide the Settings status labels and exact redacted message. See [Extras guide](../account-extras/index.md), [configuration reference](../configuration-reference/index.md), and [channel playback](../troubleshooting-channels-and-playback/index.md).
