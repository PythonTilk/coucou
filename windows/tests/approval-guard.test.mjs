// Which click may answer a card (src/core/approval-guard.ts).

import test from "node:test";
import assert from "node:assert/strict";

const { clickedRequest, CLICK_GUARD_MS } = await import("../src/core/approval-guard.ts");
const { State } = await import("../src/core/state.ts");

const req = (requestId) => ({ requestId, sessionId: "s", pillId: "integration_claude", tool: "Bash", command: "Bash · ls" });

test("a click answers only the request it was drawn for", () => {
  const up = req("r2");
  const late = 10_000;
  assert.equal(clickedRequest(up, "r2", 0, late), up);
  // The card moved on to the next request under a click meant for the last one.
  assert.equal(clickedRequest(up, "r1", 0, late), null);
  assert.equal(clickedRequest(up, null, 0, late), null);
  assert.equal(clickedRequest(null, "r2", 0, late), null);
});

test("a card that has just come up takes no click yet", () => {
  const up = req("r1");
  assert.equal(clickedRequest(up, "r1", 1000, 1000 + CLICK_GUARD_MS - 1), null);
  assert.equal(clickedRequest(up, "r1", 1000, 1000 + CLICK_GUARD_MS), up);
});

test("a card's guard starts when it comes up, and not again for the same request", () => {
  State.tasks = [];
  State.pendingApproval = null;
  State.approvalShownAt = 0;
  State.beginApproval(req("r1"));
  const first = State.approvalShownAt;
  assert.ok(first > 0);
  State.beginApproval(req("r1"));
  assert.equal(State.approvalShownAt, first);
  // The next request starts its own.
  State.pendingApproval = null;
  State.approvalShownAt = 0;
  State.beginApproval(req("r2"));
  assert.ok(State.approvalShownAt > 0);
  State.pendingApproval = null;
  State.isPinned = false;
});
