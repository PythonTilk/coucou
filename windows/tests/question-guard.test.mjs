// The question card on a fake DOM: a click that does not count changes nothing
// (src/views/views.ts, core/approval-guard.ts). A card with several questions
// must not have its first one ticked off by a click the guard then refuses.

import { beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { buildQuestion } = await import("../src/views/views.ts");
const { CLICK_GUARD_MS } = await import("../src/core/approval-guard.ts");
const { DEFAULT_SETTINGS, State } = await import("../src/core/state.ts");

/** What the card asked the island to do. */
let sent;
const actions = new Proxy(
  {
    answer: (answers, drawnFor) => sent.push({ answers, drawnFor }),
    answerInTerminal: (drawnFor) => sent.push({ terminal: drawnFor }),
  },
  { get: (target, key) => target[key] ?? (() => {}) },
);
const view = buildQuestion(actions);

/** Every element under `el` that takes a click, in document order. */
function buttons(el, out = []) {
  for (const child of el.children ?? []) {
    if (child.tagName === "BUTTON" && String(child.className).includes("btn ")) out.push(child);
    buttons(child, out);
  }
  return out;
}
const label = (b) => b.children[0]?.textContent;
const click = (text) => buttons(view.el).find((b) => label(b) === text).fire("click");

const TWO = [
  { question: "First?", multiSelect: false, options: [{ label: "A", description: "" }, { label: "B", description: "" }] },
  { question: "Second?", multiSelect: false, options: [{ label: "C", description: "" }, { label: "D", description: "" }] },
];

function show(requestId, shownAgoMs) {
  State.pendingApproval = {
    requestId, sessionId: "s", pillId: "integration_claude", tool: "AskUserQuestion", command: "", questions: TWO,
  };
  State.approvalShownAt = performance.now() - shownAgoMs;
  view.sync();
}

beforeEach(() => {
  sent = [];
  State.settings = { ...DEFAULT_SETTINGS };
  State.tasks = [];
  State.loadIntegrationTasks();
  State.approvalQueue = [];
});

test("a click on a card that has just come up ticks nothing off", () => {
  show("q1", 0);
  click("A");
  view.sync();
  // Still on the first question: the click was not noted.
  assert.deepEqual(buttons(view.el).map(label).slice(0, 2), ["A", "B"]);
  assert.deepEqual(sent, []);
});

test("once it can be read, the answers go out together, for the request they were given to", () => {
  show("q2", CLICK_GUARD_MS + 1);
  click("B");
  view.sync();
  assert.deepEqual(buttons(view.el).map(label).slice(0, 2), ["C", "D"]);
  click("C");
  assert.deepEqual(sent, [{ answers: { "First?": "B", "Second?": "C" }, drawnFor: "q2" }]);
});

test("a click meant for the card before changes nothing on the one that took its place", () => {
  show("q3", CLICK_GUARD_MS + 1);
  const stale = buttons(view.el).find((b) => label(b) === "A");
  // The next request takes the card before the page has drawn it.
  State.pendingApproval = { ...State.pendingApproval, requestId: "q4" };
  State.approvalShownAt = performance.now() - (CLICK_GUARD_MS + 1);
  stale.fire("click");
  view.sync();
  assert.deepEqual(buttons(view.el).map(label).slice(0, 2), ["A", "B"]);
  assert.deepEqual(sent, []);
});
