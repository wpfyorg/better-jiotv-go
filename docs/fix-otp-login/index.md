---
title: Fix JioTV OTP Login
description: Recover when a JioTV code fails, expires or your account stays disconnected.
category: Troubleshooting
keywords: [OTP, login, phone, sign in, code expired, authentication, session]
---

# Fix JioTV OTP Login

## Symptoms
The OTP never arrives, a code is rejected, or the JioTV card remains **Not logged in** after you finish signing in.

## Possible causes
The number or code is wrong, the code expired, the provider rejected the request, the account requires another verification step, or the stored session is no longer valid.

## Diagnosis
1. In **Settings**, check the **JioTV** card and whether it reports an account connection.
2. Confirm that the OTP went to the number associated with your own account. Do not post it or paste it into issue reports.
3. Check the message displayed by JioTV and the server log at that time. Do not mistake a browser admin-password prompt for provider OTP login.

## Solution
1. Start a fresh sign-in through the JioTV card or run `jiotv login otp` in a terminal.
2. Enter the most recent code before it expires. Avoid requesting several codes rapidly.
3. If a session is invalid, follow the [account-session guide](../account-sessions/index.md) for the available reset and reconnect actions.
4. If the provider is unavailable, wait until it accepts login again.

## Verification
The JioTV card should show **Logged in**; then visit **Channels** and confirm the catalogue loads. Successful login does not guarantee access to every subscription channel.

## Further help
Provide a redacted error, login method, app version, time and account status only. Never share OTP, password, cookie, token or phone number. See [connect an account](../account-connect-jiotv/index.md) and [account troubleshooting](../troubleshooting-accounts-and-extras/index.md).
