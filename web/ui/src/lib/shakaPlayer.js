import { loadScript } from "./loadScript.js";

let stylesheetPromise = null;

function loadStylesheet(href) {
  if (!stylesheetPromise) {
    stylesheetPromise = new Promise((resolve, reject) => {
      const existing = document.querySelector(`link[href="${href}"]`);
      if (existing) {
        resolve();
        return;
      }
      const link = document.createElement("link");
      link.rel = "stylesheet";
      link.href = href;
      link.onload = resolve;
      link.onerror = () => {
        stylesheetPromise = null;
        reject(new Error("could not load " + href));
      };
      document.head.appendChild(link);
    });
  }
  return stylesheetPromise;
}

export async function widevineCapability() {
  if (typeof navigator.requestMediaKeySystemAccess !== "function") {
    return { usable: false, status: "eme_unavailable", message: "Encrypted Media Extensions are unavailable in this browser context." };
  }

  let access;
  try {
    access = await navigator.requestMediaKeySystemAccess("com.widevine.alpha", [
      {
        initDataTypes: ["cenc"],
        videoCapabilities: [{ contentType: 'video/mp4; codecs="avc1.42E01E"' }],
      },
    ]);
  } catch (err) {
    return { usable: false, status: "key_system_unavailable", message: err?.message || "Widevine key-system access is unavailable." };
  }

  let mediaKeys;
  try {
    mediaKeys = await access.createMediaKeys();
  } catch (err) {
    return {
      usable: false,
      status: "cdm_unavailable",
      message: err?.message || "Widevine is advertised by the browser, but no usable CDM could be created.",
    };
  }

  let session;
  try {
    session = mediaKeys.createSession("temporary");
  } catch (err) {
    return {
      usable: false,
      status: "cdm_unavailable",
      message: err?.message || "Widevine MediaKeys were created, but no usable MediaKeySession could be constructed.",
    };
  }

  session.close?.().catch(() => {});
  return { usable: true, status: "available", message: "Widevine CDM session construction succeeded." };
}

export function isDrmPlaybackError(error) {
  const category = error?.category;
  const code = Number(error?.code);
  return category === 6 || (Number.isFinite(code) && code >= 6000 && code < 7000);
}

export function playbackErrorMessage(error, capability = null) {
  const code = error?.code;
  if (isDrmPlaybackError(error) && capability && !capability.usable) {
    return `DRM_ENVIRONMENT_BLOCKED: ${capability.message}`;
  }
  if (code !== undefined && code !== null) return "Playback error " + code;
  return error?.message || String(error || "Playback failed");
}

// Shaka reports a failed license request as LICENSE_REQUEST_FAILED (6007) with
// the underlying network error in data[0]; the HTTP status lives on that one.
function unwrapLicenseError(error) {
  const inner = Number(error?.code) === 6007 ? error?.data?.[0] : null;
  return inner && typeof inner === "object" ? inner : error;
}

// Shaka BAD_HTTP_STATUS (1001) carries [uri, status, ...] in error.data.
export function playbackHttpStatus(error) {
  error = unwrapLicenseError(error);
  const status = error?.httpStatus ?? error?.data?.httpStatus ?? (Number(error?.code) === 1001 ? error?.data?.[1] : undefined);
  const n = Number(status);
  return Number.isFinite(n) ? n : null;
}

// Decide which user-facing explanation fits a terminal live-playback failure:
// "browser_unsupported" (DRM/key-system capability and no usable HLS),
// "provider_unavailable" (upstream 404) or "generic" (show the raw error).
export function classifyPlaybackFailure({ dashError = null, hlsError = null, hadHls = false, capability = null, secureContext = true } = {}) {
  // Browsers expose EME (Widevine) and Web Crypto (AES-128 HLS, Shaka 4042)
  // only on HTTPS or localhost, so plain-HTTP LAN access fails before any
  // codec question arises.
  const needsSecure = (error) => !!error && (isDrmPlaybackError(error) || Number(error.code) === 4042);
  if (!secureContext && (needsSecure(dashError) || needsSecure(hlsError))) return "insecure_context";
  const drmCause = (error) =>
    !!error && isDrmPlaybackError(error) && (Number(error.code) === 6001 || (!!capability && !capability.usable));
  if (drmCause(dashError) && (!hadHls || hlsError)) return "browser_unsupported";
  const status = playbackHttpStatus(hlsError ?? dashError);
  if (status === 404) return "provider_unavailable";
  if (status === 401 || status === 403) return "provider_denied";
  return "generic";
}

// `/api/live/play` answers 404 "No stream found..." when the provider returned no
// usable DASH or HLS source; that is the same provider-side outage a 404 from
// the player reports. Other 404s (e.g. a channel not available for the active
// account) stay generic.
export function sourceResolutionFailure(error) {
  const noStream = Number(error?.status) === 404 && /no stream found/i.test(error?.message ?? "");
  if (noStream) return "provider_unavailable";
  return sourceDenialStatus(error) ? "provider_denied" : "generic";
}

// The provider's own refusal (HTTP 401/403) behind a failed source request.
// `/api/live/play` reports an upstream refusal as a server error whose message
// carries the provider's status line, e.g. "HTTP status client error (403
// Forbidden) for url (...)"; a proper 403 from this server counts too. Returns
// the status, or null when the failure is anything else.
export function sourceDenialStatus(error) {
  if (Number(error?.status) === 403) return 403;
  const message = error?.message ?? "";
  const m = /\((40[13])\s+[A-Za-z ]+\)/.exec(message);
  if (m) return Number(m[1]);
  // The extra source's playback API answers an unsubscribed channel with a 401
  // that the server turns into this sentence, dropping the status line.
  if (/is not in your extras plan/i.test(message)) return 401;
  return null;
}

export async function loadLiveSource({ player, video, source, drmCapability = null, secureContext = globalThis.isSecureContext ?? true, isCurrent = () => true, allowMutedFallback = true, onAutoplayResult = () => {}, onTerminalError = () => {} }) {
  let fallbackPromise = null;
  let usingHls = false;
  let fallbackSettled = false;
  let terminalReported = false;
  let dashError = null;
  let hlsError = null;

  // onTerminalError(message, { kind, dashError, hlsError, hadHls }); the second
  // argument is optional for callers that only need the message.
  const reportTerminal = (error) => {
    if (!terminalReported && isCurrent()) {
      terminalReported = true;
      if (usingHls) hlsError = error;
      else dashError = dashError ?? error;
      const hadHls = !!(source.dash && source.hls);
      const kind = classifyPlaybackFailure({ dashError, hlsError, hadHls, capability: drmCapability, secureContext });
      // A browser-unsupported failure is explained by the DASH/DRM cause, not by
      // whatever the HLS alternative then failed with.
      const reported = kind === "browser_unsupported" && dashError ? dashError : error;
      onTerminalError(playbackErrorMessage(reported, drmCapability), { kind, dashError, hlsError, hadHls });
    }
  };

  const fallbackToHls = () => {
    if (!source.dash || !source.hls) return Promise.resolve(false);
    if (!isCurrent()) return Promise.resolve(false);
    if (fallbackPromise) return fallbackPromise;

    fallbackPromise = (async () => {
      usingHls = true;
      try {
        await player.unload();
        if (!isCurrent()) return false;
        try {
          await player.load(source.hls);
        } catch (error) {
          if (error?.code !== 4032 || !player.getConfiguration().drm.servers["com.widevine.alpha"]) throw error;
          // Shaka applies configured license servers even to HLS without DRM
          // metadata. Retry without the DASH server so AES HLS can use WebCrypto.
          await player.unload();
          if (!isCurrent()) return false;
          player.configure({ drm: { servers: undefined } });
          await player.load(source.hls);
        }
        if (!isCurrent()) return false;
        const played = await playWithAutoplay(video, { allowMutedFallback });
        if (isCurrent()) onAutoplayResult(played);
        return true;
      } finally {
        fallbackSettled = true;
      }
    })();
    return fallbackPromise;
  };

  player.addEventListener("error", (event) => {
    const detail = event.detail;
    // Shaka RECOVERABLE (1) errors are reported while it keeps playing, for any
    // source; only CRITICAL ones may fall back to HLS or end playback.
    if (detail?.severity === 1) return;
    if (!usingHls) dashError = dashError ?? detail;
    if (source.dash && source.hls && !usingHls) {
      fallbackToHls().catch(reportTerminal);
      return;
    }
    if (usingHls && !fallbackSettled) return;
    reportTerminal(detail);
  });

  try {
    await player.load(source.url);
  } catch (error) {
    dashError = dashError ?? error;
    if (source.dash && source.hls) {
      try {
        if (await fallbackToHls()) return;
        if (!isCurrent()) return;
      } catch (fallbackError) {
        reportTerminal(fallbackError);
        return;
      }
    }
    reportTerminal(error);
    return;
  }

  if (fallbackPromise) {
    try {
      await fallbackPromise;
    } catch (error) {
      reportTerminal(error);
    }
    return;
  }

  if (isCurrent()) {
    const played = await playWithAutoplay(video, { allowMutedFallback });
    if (isCurrent()) onAutoplayResult(played);
  }
}

export async function createShakaPlayer(container, video, { controls = true } = {}) {
  await Promise.all([
    loadScript("/static/external/shaka-player.ui.js"),
    loadStylesheet("/static/external/shaka-player-controls.css"),
  ]);

  const shaka = window.shaka;
  shaka.polyfill.installAll();
  if (!shaka.Player.isBrowserSupported()) {
    throw new Error("This browser is not supported by Shaka Player.");
  }

  const player = new shaka.Player();
  await player.attach(video);
  const ui = new shaka.ui.Overlay(player, container, video);
  ui.configure(controls ? {
    addBigPlayButton: false,
    fadeDelay: 3,
    enableKeyboardPlaybackControls: true,
    enableTooltips: true,
    singleClickForPlayAndPause: true,
    doubleClickForFullscreen: true,
    controlPanelElements: ["play_pause", "time_and_duration", "spacer", "mute", "volume", "quality", "fullscreen", "overflow_menu"],
    overflowMenuButtons: ["captions", "language", "picture_in_picture", "playback_rate"],
    seekBarColors: {
      base: "rgba(255,255,255,.22)",
      buffered: "rgba(255,255,255,.48)",
      played: "#5b8cff",
    },
    volumeBarColors: {
      base: "rgba(255,255,255,.28)",
      level: "#ffffff",
    },
  } : {
    addBigPlayButton: false,
    addSeekBar: false,
    fadeDelay: 3,
    enableKeyboardPlaybackControls: false,
    enableTooltips: false,
    singleClickForPlayAndPause: false,
    doubleClickForFullscreen: false,
    controlPanelElements: [],
    overflowMenuButtons: [],
  });

  player.configure({
    manifest: { dash: { clockSyncUri: "/dashtime" } },
    streaming: {
      retryParameters: { maxAttempts: 2, backoffFactor: 2, timeout: 30000 },
      bufferBehind: 5,
      bufferingGoal: 15,
      rebufferingGoal: 2,
    },
  });

  return {
    player,
    destroy: async () => {
      await ui.destroy();
      await player.destroy();
    },
  };
}

export async function playWithAutoplay(video, { allowMutedFallback = true } = {}) {
  video.muted = false;
  try {
    await video.play();
    return true;
  } catch {
    if (!allowMutedFallback) return false;
    video.muted = true;
    try {
      await video.play();
      return true;
    } catch {
      return false;
    }
  }
}
