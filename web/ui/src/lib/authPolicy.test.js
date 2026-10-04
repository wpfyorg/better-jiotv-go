import assert from "node:assert/strict";
import test from "node:test";

import { signsOutOn401 } from "./authPolicy.js";

test("a rejected password or unlock code does not sign the viewer out", () => {
  assert.equal(signsOutOn401("/api/auth/login"), false);
  assert.equal(signsOutOn401("/api/extras/unlock"), false);
});

test("a 401 from any other endpoint means the session is gone", () => {
  assert.equal(signsOutOn401("/api/channels"), true);
  assert.equal(signsOutOn401("/api/live/play/144?q=auto"), true);
  assert.equal(signsOutOn401("/api/extras/lock"), true);
});
