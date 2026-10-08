// Integration events → island state. Port of the `handle…` methods in the Swift
// pollers: a genuinely new item flips the pill to finished/error, badges it when
// the pill isn't focused, plays a sound, and clears itself after 60 s.

import { onEvent, Bridge, type IntegrationUpdate } from "../core/bridge";
import { gitHubAlert, type GitHubEvent } from "../core/github";
import { availablePills } from "../core/pills";
import { Sound } from "../core/sound";
import { State } from "../core/state";
import type { Island } from "./island";

const GITHUB = "integration_github";

const clearTimers = new Map<string, number>();

const CALENDAR_ID = "integration_calendar";
/** How long before something starts the island says so. */
const NUDGE_MINUTES = 10;
/** Events already announced, so each is announced once. */
const nudged = new Set<string>();
let nudgeClock: number | null = null;

/** Reveals the island when a lecture or meeting is about to start. (Local to this build.) */
function nudgeForCalendar(island: Island) {
  if (State.paused) return;
  const task = State.tasks.find((t) => t.id === CALENDAR_ID);
  const events = State.integrations[CALENDAR_ID]?.data?.events;
  if (!task || !Array.isArray(events)) return;

  for (const event of events as { startMs: number; title: string; location?: string | null; allDay: boolean }[]) {
    if (event.allDay) continue;
    const minutes = (event.startMs - Date.now()) / 60_000;
    const key = `${event.startMs}:${event.title}`;
    if (minutes > NUDGE_MINUTES || minutes < -1 || nudged.has(key)) continue;
    nudged.add(key);

    const soon = minutes < 1 ? "now" : `in ${Math.round(minutes)} min`;
    task.steps = [`${event.title} ${soon}`, ...(event.location ? [event.location] : [])];
    task.stepIndex = 0;
    if (State.focusId !== CALENDAR_ID) task.pillBadge = "approval";
    Sound.play("question");
    island.reveal();
    State.notify();
  }
}

export function registerIntegrationHandlers(island: Island) {
  void onEvent<IntegrationUpdate>("integration", (update) => {
    handle(island, update);
    // The nudge clock only starts once a calendar has answered: without one
    // there is nothing to announce, and no timer left running for nothing.
    if (update.id === CALENDAR_ID && nudgeClock == null) {
      nudgeClock = window.setInterval(() => nudgeForCalendar(island), 30_000);
    }
  });
  void onEvent<GitHubEvent[]>("github-alerts", handleGitHubAlerts);
  void refreshConfigured();
  State.subscribe(refreshGitHubWhenShown);
}

/**
 * AppState.handleGitHubEvents: the loudest event sets the badge — only while the
 * GitHub pill isn't the one on screen — and plays its sound. Unlike the other
 * integrations it leaves Mochi's state alone and the badge stays until the pill
 * is focused, as on macOS.
 */
export function handleGitHubAlerts(events: GitHubEvent[]) {
  if (State.paused) return;
  const alert = gitHubAlert(events);
  if (!alert) return;
  const task = State.tasks.find((t) => t.id === GITHUB);
  if (!task) return;
  if (State.focusId !== GITHUB) task.pillBadge = alert.badge;
  Sound.play(alert.sound);
  State.notify();
}

let gitHubShown = false;

/** The GitHub card just came on screen (focused, island opened): refresh it if stale. */
function refreshGitHubWhenShown() {
  const shown = State.mode === "expanded" && State.focusTask?.id === GITHUB;
  if (shown && !gitHubShown) void Bridge.githubRefresh("pulse");
  gitHubShown = shown;
}

/**
 * Whether each pill is connected, so its idle card can say so: a key in the
 * credential store for a service or a chat provider, the hooks in place for a
 * pill fed by hook events (Mac #183), nothing at all for Claude Desktop.
 */
export async function refreshConfigured() {
  const hooks = (await Bridge.agentHooksStatus()) ?? null;
  for (const def of availablePills(State.os)) {
    let configured: boolean;
    switch (def.connect.kind) {
      case "key":
        configured = (await Bridge.secretPresent(def.connect.key)) ?? false;
        break;
      case "hooks":
        // Without an answer from Rust (a plain browser), the Claude Code pill
        // falls back to what the settings say about its hooks.
        configured = hooks?.[def.id] ??
          (def.id === "integration_claude" ? State.settings.hooksInstalled : false);
        break;
      case "server":
        // A local model server counts once the chat is connected to it.
        configured = State.settings[def.connect.field] !== "";
        break;
      case "none":
        configured = true;
        break;
    }
    const info = State.integrations[def.id] ?? { data: {}, error: null, loaded: false, configured: false };
    State.integrations[def.id] = { ...info, configured };
  }
  State.notify();
}

/** Only the hook-driven pills, for when the island opens: a few small file reads. */
export async function refreshHookPills() {
  const hooks = await Bridge.agentHooksStatus();
  if (!hooks) return;
  for (const [id, present] of Object.entries(hooks)) {
    const info = State.integrations[id] ?? { data: {}, error: null, loaded: false, configured: false };
    State.integrations[id] = { ...info, configured: present };
  }
  State.notify();
}

function handle(island: Island, update: IntegrationUpdate) {
  if (State.paused) return;

  const previous = State.integrations[update.id];
  State.integrations[update.id] = {
    data: update.error ? (previous?.data ?? {}) : update.data,
    error: update.error,
    loaded: update.error ? (previous?.loaded ?? false) : true,
    configured: previous?.configured ?? true,
  };

  const event = update.event;
  if (event) {
    const task = State.tasks.find((t) => t.id === update.id);
    if (task) {
      task.state = event.success ? "finished" : "error";
      task.steps = event.detail ? [event.label, event.detail] : [event.label];
      task.stepIndex = task.steps.length - 1;
      if (State.focusId !== update.id) {
        task.pillBadge = event.success ? "finished" : "error";
      }
      Sound.play(event.success ? "finish" : "error");
      // Same as the Swift pollers: show the compact island so the badge is seen,
      // but never steal the screen for a successful deploy.
      island.reveal();

      const existing = clearTimers.get(update.id);
      if (existing != null) window.clearTimeout(existing);
      clearTimers.set(
        update.id,
        window.setTimeout(() => {
          clearTimers.delete(update.id);
          const t = State.tasks.find((x) => x.id === update.id);
          if (!t || (t.state !== "finished" && t.state !== "error")) return;
          t.state = "idle";
          t.steps = [];
          t.stepIndex = 0;
          t.pillBadge = null;
          State.notify();
        }, 60_000),
      );
    }
  }

  State.notify();
}
