// Settings → Updates (this build): the line that says where things stand.

import test from "node:test";
import assert from "node:assert/strict";

const { statusLine, checkedLabel } = await import("../src/settings/updates.ts");

const NOW = 1_800_000_000_000;
const base = { current: "0.4.3", available: null, notes: null, checkedAt: 0, error: null, installing: false };

test("the update line says what is known, most pressing first", () => {
  assert.equal(statusLine(null, NOW), "Updates are not available in this window.");
  assert.equal(statusLine(base, NOW), "Not checked yet.");
  assert.equal(
    statusLine({ ...base, checkedAt: NOW / 1000 - 5 * 60 }, NOW),
    "This is the newest build. Checked 5 min ago.",
  );
  assert.equal(statusLine({ ...base, error: "offline", checkedAt: NOW / 1000 }, NOW), "Could not check: offline");
  // A build that was found stays on offer when a later look fails.
  assert.equal(
    statusLine({ ...base, available: "0.4.7", error: "offline", checkedAt: NOW / 1000 }, NOW),
    "Version 0.4.7 is ready to install.",
  );
  assert.equal(
    statusLine({ ...base, available: "0.4.7", installing: true }, NOW),
    "Installing… Coucou will close and come back.",
  );
});

test("when the last look was is said in the nearest unit", () => {
  assert.equal(checkedLabel(0, NOW), "Not checked yet");
  assert.equal(checkedLabel(NOW / 1000 - 20, NOW), "Checked just now");
  assert.equal(checkedLabel(NOW / 1000 - 59 * 60, NOW), "Checked 59 min ago");
  assert.equal(checkedLabel(NOW / 1000 - 3 * 3600, NOW), "Checked 3 h ago");
  assert.match(checkedLabel(NOW / 1000 - 3 * 86400, NOW), /^Checked on /);
});
