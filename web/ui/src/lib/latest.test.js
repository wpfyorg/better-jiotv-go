import assert from "node:assert/strict";
import test from "node:test";

import { latestOnly } from "./latest.js";

test("only the most recently started request may publish", () => {
  const loads = latestOnly();
  const first = loads.start();
  assert.equal(first(), true);
  const second = loads.start();
  assert.equal(first(), false, "the earlier request must be superseded");
  assert.equal(second(), true);
  const third = loads.start();
  assert.equal(second(), false);
  assert.equal(third(), true);
});
