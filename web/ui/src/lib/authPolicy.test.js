import assert from "node:assert/strict";
import test from "node:test";

import { signsOutOn401 } from "./authPolicy.js";

test("a rejected password or unlock code does not sign the viewer out", () => {
  assert.equal(signsOutOn401("/api/auth/login", "wrong password"), false);
  assert.equal(signsOutOn401("/api/extras/unlock", "wrong code"), false);
});

test("an expired session still signs the viewer out on the unlock endpoint", () => {
  // The access gate's own 401 is plain text, not the endpoint's "wrong code".
  assert.equal(signsOutOn401("/api/extras/unlock", "unauthorized"), true);
  assert.equal(signsOutOn401("/api/extras/unlock", "invalid access key"), true);
  assert.equal(signsOutOn401("/api/extras/unlock"), true);
});

test("a 401 from any other endpoint means the session is gone", () => {
  assert.equal(signsOutOn401("/api/channels"), true);
  assert.equal(signsOutOn401("/api/live/play/144?q=auto"), true);
  assert.equal(signsOutOn401("/api/extras/lock"), true);
});
