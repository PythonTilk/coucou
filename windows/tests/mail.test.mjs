// The Mail card's small pieces (src/views/integrations.ts): how old a message
// is said to be, and where a click on it goes.

import test from "node:test";
import assert from "node:assert/strict";
import { installFakeDom } from "./fakedom.mjs";

installFakeDom();
const { mailAge, mailLink } = await import("../src/views/integrations.ts");

const NOW = 1_800_000_000_000;
const MIN = 60_000;

test("a message's age is said in the nearest unit", () => {
  assert.equal(mailAge(0, NOW), "");
  assert.equal(mailAge(NOW - 20_000, NOW), "now");
  assert.equal(mailAge(NOW - 12 * MIN, NOW), "12 min");
  assert.equal(mailAge(NOW - 3 * 60 * MIN, NOW), "3 h");
  // Days old: a weekday, then a date; never a count that keeps growing.
  assert.doesNotMatch(mailAge(NOW - 2 * 24 * 60 * MIN, NOW), /\d+ (min|h)/);
  assert.match(mailAge(NOW - 30 * 24 * 60 * MIN, NOW), /\d/);
});

test("a click goes to the conversation in Gmail, and nowhere for another server", () => {
  assert.equal(mailLink("18f3a2c4d5e6f701", true), "https://mail.google.com/mail/u/0/#inbox/18f3a2c4d5e6f701");
  // No id, or something that is not one: the inbox, never a link built from it.
  assert.equal(mailLink(undefined, true), "https://mail.google.com/mail/u/0/#inbox");
  assert.equal(mailLink("../../evil?x=1", true), "https://mail.google.com/mail/u/0/#inbox");
  assert.equal(mailLink("18f3", false), null);
});
