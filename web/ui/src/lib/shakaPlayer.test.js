import assert from "node:assert/strict";
import test from "node:test";

import { classifyPlaybackFailure, loadLiveSource, sourceResolutionFailure } from "./shakaPlayer.js";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function fakePlayer(loadImpl) {
  let errorHandler;
  return {
    loads: [],
    unloads: 0,
    config: { drm: { servers: { "com.widevine.alpha": "license" } } },
    getConfiguration() { return this.config; },
    configure(config) {
      if (Object.hasOwn(config.drm, "servers")) this.config.drm.servers = config.drm.servers ?? {};
    },
    addEventListener(type, handler) {
      if (type === "error") errorHandler = handler;
    },
    emitError(detail) {
      errorHandler?.({ detail });
    },
    async unload() {
      this.unloads++;
    },
    async load(url) {
      this.loads.push(url);
      return loadImpl(url, this);
    },
  };
}

const video = { muted: false, play: async () => {} };

test("retries unsupported HLS without the inherited DASH license server", async () => {
  const player = fakePlayer((url, p) => {
    if (url === "dash") return Promise.reject({ code: 6001 });
    if (p.config.drm.servers["com.widevine.alpha"]) return Promise.reject({ code: 4032 });
    return Promise.resolve();
  });
  const errors = [];
  await loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: "hls" }, onTerminalError: e => errors.push(e) });
  assert.deepEqual(player.loads, ["dash", "hls", "hls"]);
  assert.equal(player.unloads, 2);
  assert.deepEqual(errors, []);
});

test("reports failure after the bounded HLS configuration retry", async () => {
  const player = fakePlayer(() => Promise.reject({ code: 4032 }));
  const errors = [];
  await loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: "hls" }, onTerminalError: e => errors.push(e) });
  assert.deepEqual(player.loads, ["dash", "hls", "hls"]);
  assert.deepEqual(errors, ["Playback error 4032"]);
});

test("keeps the license server when provider HLS loads successfully", async () => {
  const player = fakePlayer(url => url === "dash" ? Promise.reject({ code: 6001 }) : Promise.resolve());
  await loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: "hls" } });
  assert.deepEqual(player.loads, ["dash", "hls"]);
  assert.equal(player.config.drm.servers["com.widevine.alpha"], "license");
});

test("does not retry unsupported HLS without an inherited license server", async () => {
  const player = fakePlayer(() => Promise.reject({ code: 4032 }));
  player.config.drm.servers = {};
  await loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: "hls" } });
  assert.deepEqual(player.loads, ["dash", "hls"]);
});

test("route cancellation prevents the HLS configuration retry", async () => {
  let current = true;
  const player = fakePlayer(() => Promise.reject({ code: 4032 }));
  player.unload = async () => { if (++player.unloads === 2) current = false; };
  const errors = [];
  await loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: "hls" }, isCurrent: () => current, onTerminalError: e => errors.push(e) });
  assert.deepEqual(player.loads, ["dash", "hls"]);
  assert.equal(player.config.drm.servers["com.widevine.alpha"], "license");
  assert.deepEqual(errors, []);
});

test("shares one HLS recovery between Shaka error event and load rejection", async () => {
  const dash = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : Promise.resolve()));
  const errors = [];
  const loading = loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    onTerminalError: (message) => errors.push(message),
  });

  await Promise.resolve();
  player.emitError({ code: 4032 });
  dash.reject({ code: 4032 });
  await loading;

  assert.deepEqual(player.loads, ["dash", "hls"]);
  assert.equal(player.unloads, 1);
  assert.deepEqual(errors, []);
});

test("suppresses duplicate Shaka errors while HLS recovery is still loading", async () => {
  const dash = deferred();
  const hls = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : hls.promise));
  const errors = [];
  const loading = loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    onTerminalError: (message) => errors.push(message),
  });

  await Promise.resolve();
  player.emitError({ code: 4032 });
  await Promise.resolve();
  player.emitError({ code: 4032 });
  dash.reject({ code: 4032 });
  hls.resolve();
  await loading;

  assert.deepEqual(player.loads, ["dash", "hls"]);
  assert.deepEqual(errors, []);
});

test("reports the HLS failure only after recovery fails", async () => {
  const dash = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : Promise.reject(new Error("HLS failed"))));
  const errors = [];
  const loading = loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    onTerminalError: (message) => errors.push(message),
  });

  await Promise.resolve();
  player.emitError({ code: 4032 });
  dash.reject({ code: 4032 });
  await loading;

  assert.deepEqual(errors, ["HLS failed"]);
});

test("keeps MPD-only playback on DASH and reports its terminal error", async () => {
  const player = fakePlayer(() => Promise.reject({ code: 6001 }));
  const errors = [];
  await loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: null },
    onTerminalError: (message) => errors.push(message),
  });

  assert.deepEqual(player.loads, ["dash"]);
  assert.equal(player.unloads, 0);
  assert.deepEqual(errors, ["Playback error 6001"]);
});

test("stale playback cannot start its HLS alternative or publish errors", async () => {
  let current = true;
  const dash = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : Promise.resolve()));
  const errors = [];
  const loading = loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    isCurrent: () => current,
    onTerminalError: (message) => errors.push(message),
  });

  await Promise.resolve();
  current = false;
  player.emitError({ code: 4032 });
  dash.reject({ code: 4032 });
  await loading;

  assert.deepEqual(player.loads, ["dash"]);
  assert.equal(player.unloads, 0);
  assert.deepEqual(errors, []);
});

test("stale playback ignores a failing HLS recovery already in flight", async () => {
  let current = true;
  const dash = deferred();
  const hls = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : hls.promise));
  const errors = [];
  const loading = loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    isCurrent: () => current,
    onTerminalError: (message) => errors.push(message),
  });

  await Promise.resolve();
  player.emitError({ code: 4032 });
  await Promise.resolve();
  assert.deepEqual(player.loads, ["dash", "hls"]);

  current = false;
  dash.reject({ code: 4032 });
  hls.reject(new Error("late HLS failure"));
  await loading;

  assert.equal(player.unloads, 1);
  assert.deepEqual(errors, []);
});

const http404 = () => ({ code: 1001, data: ["https://example.invalid/x.m3u8", 404] });

test("classifies DRM key-system failure with no or failed HLS as browser_unsupported", () => {
  assert.equal(classifyPlaybackFailure({ dashError: { code: 6001 }, hadHls: false }), "browser_unsupported");
  assert.equal(classifyPlaybackFailure({ dashError: { code: 6001 }, hlsError: http404(), hadHls: true }), "browser_unsupported");
  assert.equal(classifyPlaybackFailure({ dashError: { code: 6006, category: 6 }, hadHls: false, capability: { usable: false } }), "browser_unsupported");
});

test("does not call other 6xxx errors unsupported when the CDM is usable", () => {
  assert.equal(classifyPlaybackFailure({ dashError: { code: 6007, category: 6 }, hadHls: false, capability: { usable: true } }), "generic");
  assert.equal(classifyPlaybackFailure({ dashError: { code: 6007, category: 6 }, hadHls: false }), "generic");
});

test("classifies HLS 404 without a DRM cause as provider_unavailable", () => {
  assert.equal(classifyPlaybackFailure({ dashError: http404(), hadHls: false }), "provider_unavailable");
  assert.equal(classifyPlaybackFailure({ dashError: { code: 4032 }, hlsError: { code: 1001, httpStatus: 404 }, hadHls: true }), "provider_unavailable");
  assert.equal(classifyPlaybackFailure({ dashError: { code: 1001, data: ["u", 500] }, hadHls: false }), "generic");
  assert.equal(classifyPlaybackFailure({}), "generic");
});

test("reports browser_unsupported when DASH is DRM-blocked and HLS returns 404", async () => {
  const player = fakePlayer((url) => (url === "dash" ? Promise.reject({ code: 6001 }) : Promise.reject(http404())));
  const reports = [];
  await loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    onTerminalError: (message, info) => reports.push([message, info.kind]),
  });
  assert.deepEqual(player.loads, ["dash", "hls"]);
  assert.deepEqual(reports, [["Playback error 6001", "browser_unsupported"]]);
});

test("reports browser_unsupported for MPD-only DRM failure", async () => {
  const player = fakePlayer(() => Promise.reject({ code: 6001 }));
  const reports = [];
  await loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: null }, onTerminalError: (m, info) => reports.push(info.kind) });
  assert.deepEqual(player.loads, ["dash"]);
  assert.deepEqual(reports, ["browser_unsupported"]);
});

test("reports provider_unavailable for HLS-only 404 and for HLS-event failures", async () => {
  const player = fakePlayer(() => Promise.reject(http404()));
  const reports = [];
  await loadLiveSource({ player, video, source: { dash: false, url: "hls" }, onTerminalError: (m, info) => reports.push(info.kind) });
  assert.deepEqual(reports, ["provider_unavailable"]);
});

test("reports provider_unavailable when non-DRM DASH failure falls back to a 404 HLS", async () => {
  const player = fakePlayer((url) => (url === "dash" ? Promise.reject({ code: 4032 }) : Promise.reject(http404())));
  const reports = [];
  await loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    onTerminalError: (m, info) => reports.push(info),
  });
  assert.equal(reports.length, 1);
  assert.equal(reports[0].kind, "provider_unavailable");
  assert.equal(reports[0].dashError.code, 4032);
});

test("keeps generic errors generic and uses capability for other 6xxx codes", async () => {
  const player = fakePlayer(() => Promise.reject({ code: 7000 }));
  const reports = [];
  await loadLiveSource({ player, video, source: { dash: false, url: "x" }, onTerminalError: (m, info) => reports.push(info.kind) });
  assert.deepEqual(reports, ["generic"]);

  const drm = fakePlayer(() => Promise.reject({ code: 6008, category: 6 }));
  const drmReports = [];
  await loadLiveSource({
    player: drm,
    video,
    source: { dash: true, url: "dash", hls: null },
    drmCapability: { usable: false, message: "no cdm" },
    onTerminalError: (m, info) => drmReports.push([m, info.kind]),
  });
  assert.deepEqual(drmReports, [["DRM_ENVIRONMENT_BLOCKED: no cdm", "browser_unsupported"]]);
});

test("keeps the DRM cause in the message when the HLS fallback then fails", async () => {
  const player = fakePlayer((url) => (url === "dash" ? Promise.reject({ code: 6001 }) : Promise.reject(http404())));
  const messages = [];
  await loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    drmCapability: { usable: false, message: "This browser has no working Widevine module." },
    onTerminalError: (message) => messages.push(message),
  });
  assert.deepEqual(messages, ["DRM_ENVIRONMENT_BLOCKED: This browser has no working Widevine module."]);
});

test("keeps DASH playing through a recoverable Shaka error", async () => {
  const dash = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : Promise.resolve()));
  const errors = [];
  const loading = loadLiveSource({
    player,
    video,
    source: { dash: true, url: "dash", hls: "hls" },
    onTerminalError: (message) => errors.push(message),
  });
  await Promise.resolve();
  player.emitError({ code: 1002, severity: 1 });
  dash.resolve();
  await loading;
  assert.deepEqual(player.loads, ["dash"]);
  assert.equal(player.unloads, 0);
  assert.deepEqual(errors, []);
});

test("falls back to HLS on a critical Shaka error", async () => {
  const dash = deferred();
  const player = fakePlayer((url) => (url === "dash" ? dash.promise : Promise.resolve()));
  const loading = loadLiveSource({ player, video, source: { dash: true, url: "dash", hls: "hls" } });
  await Promise.resolve();
  player.emitError({ code: 1002, severity: 2 });
  dash.resolve();
  await loading;
  assert.deepEqual(player.loads, ["dash", "hls"]);
});

test("classifies a no-stream 404 from source resolution as provider_unavailable", () => {
  assert.equal(sourceResolutionFailure({ status: 404, message: "No stream found for channel id: 154" }), "provider_unavailable");
  assert.equal(sourceResolutionFailure({ status: 404, message: "Channel 154 is not available for the active account" }), "generic");
  assert.equal(sourceResolutionFailure({ status: 500, message: "No stream found" }), "generic");
  assert.equal(sourceResolutionFailure(new Error("network down")), "generic");
});

test("ignores recoverable Shaka errors on HLS-only and fallen-back playback", async () => {
  for (const source of [{ dash: false, url: "hls" }, { dash: true, url: "dash", hls: "hls" }]) {
    const settle = deferred();
    const player = fakePlayer((url) => (url === "dash" ? Promise.reject({ code: 4032 }) : settle.promise));
    const reports = [];
    const loading = loadLiveSource({ player, video, source, onTerminalError: (message) => reports.push(message) });
    for (let i = 0; i < 5; i++) await Promise.resolve();
    player.emitError({ code: 1002, severity: 1 });
    settle.resolve();
    await loading;
    assert.deepEqual(reports, [], `source ${source.url}`);
  }
});
