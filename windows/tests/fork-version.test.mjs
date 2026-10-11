// The version a fork build is published under (scripts/fork-version.mjs).

import test from "node:test";
import assert from "node:assert/strict";

const { forkVersion } = await import("../scripts/fork-version.mjs");

test("a fork build's number rides in the third field, on top of upstream's", () => {
  assert.equal(forkVersion("0.4.0", 7), "0.4.7");
  assert.equal(forkVersion("0.4.1", 12), "0.4.1012");
  assert.equal(forkVersion("1.2.3", 999), "1.2.3999");
});

test("each build is newer than the last, across upstream versions too", () => {
  const order = [["0.4.0", 7], ["0.4.0", 8], ["0.4.1", 9], ["0.5.0", 10], ["1.0.0", 11]].map(([u, b]) =>
    forkVersion(u, b).split(".").map(Number),
  );
  for (let i = 1; i < order.length; i++) {
    const [a, b] = [order[i - 1], order[i]];
    const newer = b[0] > a[0] || (b[0] === a[0] && (b[1] > a[1] || (b[1] === a[1] && b[2] > a[2])));
    assert.ok(newer, `${b.join(".")} should be newer than ${a.join(".")}`);
  }
});

test("what cannot be a Windows Installer version is refused", () => {
  assert.throws(() => forkVersion("0.4.0-beta", 1));
  assert.throws(() => forkVersion("0.4.0", 0));
  assert.throws(() => forkVersion("0.4.0", 1000));
  assert.throws(() => forkVersion("0.4.66", 1));
});
