// Which click may answer a card.
//
// Requests wait in line, so the card under the pointer can change from one
// request to the next in the time of a click. A click therefore says which
// request it was drawn for, and counts only if that request is still the one
// that is up and has been up long enough to be read. Without this, a second
// click on Allow, or one already on its way, would allow the next request
// unread.

import type { ApprovalInfo } from "./state";

/** A card takes no click for this long after it comes up. */
export const CLICK_GUARD_MS = 600;

/**
 * The request a click may answer, or null. `drawnFor` is the request the
 * clicked control was drawn for, `shownAt` when the card that is up came up.
 */
export function clickedRequest(
  pending: ApprovalInfo | null,
  drawnFor: string | null,
  shownAt: number,
  now: number,
): ApprovalInfo | null {
  if (!pending || drawnFor == null || pending.requestId !== drawnFor) return null;
  if (now - shownAt < CLICK_GUARD_MS) return null;
  return pending;
}
