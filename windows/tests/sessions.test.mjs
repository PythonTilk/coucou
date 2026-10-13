// The live sessions behind a pill (src/island/sessions.ts).

import test from "node:test";
import assert from "node:assert/strict";

const { PillSessions, MID_TURN_SILENCE_MS, SESSION_SILENCE_MS } = await import("../src/island/sessions.ts");

const P = "integration_claude";
const about = (name) => ({ projectName: name, cwd: `/p/${name}` });

test("only an event a running session sends adds it", () => {
  const r = new PillSessions();
  r.seen(P, "a", 0, about("a"), false);
  assert.deepEqual(r.live(P, 0), []);
  r.seen(P, "a", 0, about("a"), true);
  assert.equal(r.live(P, 0).length, 1);
  // An event trailing in after the end does not bring it back.
  r.end(P, "a");
  r.seen(P, "a", 5, about("a"), false);
  assert.deepEqual(r.live(P, 5), []);
});

test("the oldest live session owns the pill, and hands it on when it ends", () => {
  const r = new PillSessions();
  r.seen(P, "a", 0, about("a"), true);
  r.seen(P, "b", 10, about("b"), true);
  assert.equal(r.owner(P, 20).id, "a");
  r.end(P, "a");
  assert.equal(r.owner(P, 20).id, "b");
  r.end(P, "b");
  assert.equal(r.owner(P, 20), null);
});

test("a sibling is busy only while it is mid-turn", () => {
  const r = new PillSessions();
  r.seen(P, "a", 0, about("a"), true);
  r.seen(P, "b", 0, about("b"), true);
  assert.equal(r.siblingMidTurn(P, "a", 1), false);
  r.turn(P, "b", true);
  assert.equal(r.siblingMidTurn(P, "a", 1), true);
  // Its own turn never counts.
  assert.equal(r.siblingMidTurn(P, "b", 1), false);
  r.turn(P, "b", false);
  assert.equal(r.siblingMidTurn(P, "a", 1), false);
});

test("a session that went silent is dropped: sooner mid-turn, later at rest", () => {
  const r = new PillSessions();
  r.seen(P, "busy", 0, about("busy"), true);
  r.turn(P, "busy", true);
  r.seen(P, "quiet", 0, about("quiet"), true);
  assert.equal(r.live(P, MID_TURN_SILENCE_MS).length, 2);
  assert.deepEqual(r.live(P, MID_TURN_SILENCE_MS + 1).map((s) => s.id), ["quiet"]);
  assert.equal(r.live(P, SESSION_SILENCE_MS).length, 1);
  assert.deepEqual(r.live(P, SESSION_SILENCE_MS + 1), []);
});

test("a session says where it is with each opening event", () => {
  const r = new PillSessions();
  r.seen(P, "a", 0, about("first"), true);
  r.seen(P, "a", 1, about("second"), false);
  assert.equal(r.owner(P, 2).projectName, "first");
  r.seen(P, "a", 2, about("second"), true);
  assert.equal(r.owner(P, 3).projectName, "second");
});

test("pills do not see each other's sessions", () => {
  const r = new PillSessions();
  r.seen(P, "a", 0, about("a"), true);
  r.turn(P, "a", true);
  assert.equal(r.siblingMidTurn("agent_codex", "x", 1), false);
  assert.deepEqual(r.live("agent_codex", 1), []);
});
