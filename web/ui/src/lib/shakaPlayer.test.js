import assert from "node:assert/strict";
import test from "node:test";

import { loadLiveSource } from "./shakaPlayer.js";

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
