import assert from "node:assert/strict";
import test from "node:test";

import { plainHttpOrigin } from "./origin.js";

const status = { httpPort: 5001, tlsPort: 5443 };

test("offers the plain-HTTP origin from the local HTTPS port", () => {
  const loc = new URL("https://192.168.1.10:5443/#/settings");
  assert.equal(plainHttpOrigin(loc, status), "http://192.168.1.10:5001");
  assert.equal(plainHttpOrigin(new URL("https://[::1]:5443/"), status), "http://[::1]:5001");
});

test("keeps the page origin on plain HTTP, tunnels and proxies", () => {
  assert.equal(plainHttpOrigin(new URL("http://192.168.1.10:5001/"), status), "http://192.168.1.10:5001");
  assert.equal(plainHttpOrigin(new URL("https://jiotv.example.com/"), status), "https://jiotv.example.com");
  assert.equal(plainHttpOrigin(new URL("https://192.168.1.10:8443/"), status), "https://192.168.1.10:8443");
});

test("keeps the page origin when the server does not report ports", () => {
  assert.equal(plainHttpOrigin(new URL("https://h:5443/"), null), "https://h:5443");
  assert.equal(plainHttpOrigin(new URL("https://h:5443/"), { httpPort: 5001, tlsPort: null }), "https://h:5443");
});

test("recognizes the default HTTPS port, which browsers report as empty", () => {
  assert.equal(plainHttpOrigin(new URL("https://192.168.1.10:443/"), { httpPort: 5001, tlsPort: 443 }), "http://192.168.1.10:5001");
  assert.equal(plainHttpOrigin(new URL("https://192.168.1.10/"), { httpPort: 5001, tlsPort: 443 }), "http://192.168.1.10:5001");
  assert.equal(plainHttpOrigin(new URL("https://192.168.1.10/"), status), "https://192.168.1.10");
});
