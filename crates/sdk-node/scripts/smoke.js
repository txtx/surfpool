const assert = require("node:assert/strict");

const { Surfnet } = require("../dist");

const keypair = Surfnet.newKeypair();

assert.equal(typeof keypair.publicKey, "string");
assert.ok(keypair.publicKey.length > 0, "expected a public key");
assert.ok(
  ArrayBuffer.isView(keypair.secretKey) || Array.isArray(keypair.secretKey),
  "expected secret key bytes"
);
assert.equal(keypair.secretKey.length, 64, "expected a 64-byte secret key");

const surfnet = Surfnet.startWithConfig({
  offline: true,
  blockProductionMode: "manual",
  report: false,
  testName: "sdk-node-smoke",
});

assert.match(surfnet.rpcUrl, /^http:\/\//);
assert.match(surfnet.wsUrl, /^ws:\/\//);
assert.ok(surfnet.payer.length > 0, "expected a payer pubkey");

console.log("sdk-node smoke test passed");
