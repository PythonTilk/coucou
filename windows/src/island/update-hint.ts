// Blue Mochi while a newer build waits. (Local to this build.)
//
// Upstream turns Mochi blue while its own updater (update.rs) has a release
// waiting; this build updates from its private repository instead
// (updates.rs, Settings → Updates), so the island asks that updater.

import { Bridge, onEvent, type UpdateStatus } from "../core/bridge";
import type { BotStateName } from "../core/layout";

/** Coucou's update blue, the colour of Mochi while a new version waits. */
export const UPDATE_BLUE = "#3B82F6";

/** Over an idle or sleeping Mochi only: anything he has to say comes first. */
export function showsUpdateHint(waiting: boolean, state: BotStateName, pending: boolean): boolean {
  return waiting && !pending && (state === "idle" || state === "sleeping");
}

let available: string | null = null;

export const UpdateHint = {
  /** A newer build is known and not being installed. */
  waiting(): boolean {
    return available !== null;
  },
  async register(): Promise<void> {
    const apply = (s: UpdateStatus | null) => {
      available = s && !s.installing ? s.available : null;
    };
    void onEvent<UpdateStatus>("update-status", apply);
    apply(await Bridge.updateStatus());
  },
};
