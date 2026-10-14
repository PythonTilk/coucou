// What the voice turn tells Claude about the island (src/island/voice.ts).

import test from "node:test";
import assert from "node:assert/strict";

const { describeIsland } = await import("../src/island/voice.ts");

const task = (over) => ({
  id: "x", name: "Claude Desktop", state: "working", steps: [], finalLine: null, isIntegration: false, ...over,
});

test("the island is described pill by pill: who, what state, the last step and words", () => {
  const text = describeIsland([
    task({ steps: ["Reads · notes.md", "Runs · npm test"] }),
    task({ name: "GitHub", state: "idle", isIntegration: true }),
    task({ name: "Codex", state: "finished", finalLine: "All tests pass." }),
  ]);
  assert.match(text, /- Claude Desktop \(agent\): working; last step: Runs · npm test/);
  assert.match(text, /- GitHub \(service\): idle$/m);
  assert.match(text, /- Codex \(agent\): finished; last words: All tests pass\./);
  assert.match(text, /^Local time: /);
});

test("an empty island says so, and long steps are cut", () => {
  assert.match(describeIsland([]), /No pills on the island\./);
  const text = describeIsland([task({ steps: ["y".repeat(500)] })]);
  assert.ok(!text.includes("y".repeat(161)));
});
