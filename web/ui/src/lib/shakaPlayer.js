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

export async function createShakaPlayer(container, video) {
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
  ui.configure({
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

export async function playWithAutoplay(video) {
  video.muted = false;
  try {
    await video.play();
  } catch {
    video.muted = true;
    await video.play().catch(() => {});
  }
}
