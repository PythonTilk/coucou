// Focus timer: a study stretch, then a break. One timer for the whole app; the
// focus view and the compact island both read it.
//
// It runs on a one-second interval rather than the island's frame loop, so it
// keeps time while the island is hidden without drawing anything.

import { State } from "./state";

export type FocusPhase = "idle" | "focus" | "break";

/** Minutes studied per day and subject, kept in the webview's own storage. */
const LOG_KEY = "coucou.focus.log";
type FocusLog = Record<string, Record<string, number>>;

export const FOCUS_PRESETS: [number, number][] = [
  [25, 5],
  [50, 10],
];

function today(): string {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function readLog(): FocusLog {
  try {
    const parsed = JSON.parse(window.localStorage.getItem(LOG_KEY) ?? "{}");
    return parsed && typeof parsed === "object" ? (parsed as FocusLog) : {};
  } catch {
    return {};
  }
}

class FocusTimer {
  phase: FocusPhase = "idle";
  focusMin = FOCUS_PRESETS[0][0];
  breakMin = FOCUS_PRESETS[0][1];
  /** What is being studied; empty is fine. */
  subject = "";

  /** Called when a stretch runs out — not when it is reset by hand. */
  onPhaseEnd: ((ended: "focus" | "break") => void) | null = null;

  private endsAt = 0;
  private pausedLeft: number | null = null;
  private interval: number | null = null;

  get running(): boolean {
    return this.phase !== "idle" && this.pausedLeft == null;
  }

  get paused(): boolean {
    return this.phase !== "idle" && this.pausedLeft != null;
  }

  private lengthMs(phase: FocusPhase): number {
    return (phase === "break" ? this.breakMin : this.focusMin) * 60_000;
  }

  remainingMs(now = Date.now()): number {
    if (this.phase === "idle") return this.lengthMs("focus");
    if (this.pausedLeft != null) return this.pausedLeft;
    return Math.max(0, this.endsAt - now);
  }

  /** Only between sessions: a running stretch keeps the length it started with. */
  setLengths(focusMin: number, breakMin: number) {
    if (this.phase !== "idle") return;
    this.focusMin = focusMin;
    this.breakMin = breakMin;
    State.notify();
  }

  start() {
    this.begin("focus");
  }

  pause() {
    if (!this.running) return;
    this.pausedLeft = this.remainingMs();
    State.notify();
  }

  resume() {
    if (this.pausedLeft == null) return;
    this.endsAt = Date.now() + this.pausedLeft;
    this.pausedLeft = null;
    State.notify();
  }

  /** Stops by hand. Whole minutes already studied still count. */
  reset() {
    if (this.phase === "focus") {
      this.record(Math.floor((this.lengthMs("focus") - this.remainingMs()) / 60_000));
    }
    this.stop();
    State.notify();
  }

  private begin(phase: "focus" | "break") {
    this.phase = phase;
    this.endsAt = Date.now() + this.lengthMs(phase);
    this.pausedLeft = null;
    if (this.interval == null) this.interval = window.setInterval(() => this.tick(), 1000);
    State.notify();
  }

  private stop() {
    this.phase = "idle";
    this.pausedLeft = null;
    if (this.interval != null) window.clearInterval(this.interval);
    this.interval = null;
  }

  private tick() {
    if (!this.running) return;
    if (this.remainingMs() > 0) {
      // Nothing to redraw while the island is out of sight.
      if (State.mode !== "hidden") State.notify();
      return;
    }
    const ended = this.phase as "focus" | "break";
    if (ended === "focus") {
      this.record(this.focusMin);
      this.begin("break");
    } else {
      this.stop();
    }
    this.onPhaseEnd?.(ended);
    State.notify();
  }

  private record(minutes: number) {
    if (minutes <= 0) return;
    try {
      const log = readLog();
      const day = (log[today()] ??= {});
      const subject = this.subject.trim();
      day[subject] = (day[subject] ?? 0) + minutes;
      window.localStorage.setItem(LOG_KEY, JSON.stringify(log));
    } catch {
      // Storage unavailable: the timer still works, it just keeps no log.
    }
  }

  /** Minutes studied today, in total and for the current subject. */
  todayMinutes(): { total: number; subject: number } {
    const day = readLog()[today()] ?? {};
    const total = Object.values(day).reduce((sum, m) => sum + m, 0);
    return { total, subject: day[this.subject.trim()] ?? 0 };
  }
}

export const Focus = new FocusTimer();

/** 1 500 000 → "25:00". */
export function formatClock(ms: number): string {
  const seconds = Math.ceil(ms / 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(Math.floor(seconds / 60))}:${pad(seconds % 60)}`;
}

/** 75 → "1 h 15 min", 40 → "40 min". */
export function formatMinutes(minutes: number): string {
  if (minutes < 60) return `${minutes} min`;
  const rest = minutes % 60;
  return rest === 0 ? `${Math.floor(minutes / 60)} h` : `${Math.floor(minutes / 60)} h ${rest} min`;
}
