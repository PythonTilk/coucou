// The live sessions behind each pill (PillSessionRegistry.swift on the Mac, #562).
//
// Two `claude` sessions under the same client — two projects in Claude Desktop,
// two terminals — share one pill. Without this, one session's SessionEnd took
// the other's card and diffs down, its Stop turned the pill to finished while
// the other was still working, and the name and folder flipped between the two.
//
// No DOM and no state of the island's own, so it can be tested by itself.

export interface LiveSession {
  id: string;
  projectName: string;
  cwd: string;
  /** When the session was first seen (epoch ms). The oldest one owns the pill's name. */
  startedAt: number;
  lastSeen: number;
  /** Between a prompt or a tool and the Stop that ends the turn. */
  midTurn: boolean;
}

/** A session mid-turn that has said nothing for this long has lost its Stop. */
export const MID_TURN_SILENCE_MS = 30 * 60_000;
/** A session that has said nothing for this long has lost its SessionEnd. */
export const SESSION_SILENCE_MS = 12 * 60 * 60_000;

export class PillSessions {
  private pills = new Map<string, Map<string, LiveSession>>();

  /** Forgets everything (tests). */
  clear() {
    this.pills.clear();
  }

  /**
   * A session shows it is alive. `opens` is true for the events that only a
   * running session sends (a start, a prompt, a tool, a request): those are the
   * ones that may add it. Anything else only refreshes a session already known,
   * so an event that trails in after SessionEnd cannot bring a ghost back.
   */
  seen(pill: string, id: string, now: number, about: { projectName: string; cwd: string }, opens: boolean) {
    let sessions = this.pills.get(pill);
    let session = sessions?.get(id);
    if (!session) {
      if (!opens) return;
      if (!sessions) this.pills.set(pill, (sessions = new Map()));
      session = { id, projectName: about.projectName, cwd: about.cwd, startedAt: now, lastSeen: now, midTurn: false };
      sessions.set(id, session);
      return;
    }
    session.lastSeen = now;
    // A session says where it is with each of these events, and may have moved.
    if (opens) {
      session.cwd = about.cwd;
      session.projectName = about.projectName;
    }
  }

  /** A turn starts or ends for a session that is known. */
  turn(pill: string, id: string, midTurn: boolean) {
    const session = this.pills.get(pill)?.get(id);
    if (session) session.midTurn = midTurn;
  }

  /** The session is over. */
  end(pill: string, id: string) {
    const sessions = this.pills.get(pill);
    if (!sessions) return;
    sessions.delete(id);
    if (sessions.size === 0) this.pills.delete(pill);
  }

  /** Every session of the pill is over (a SessionEnd that does not say whose it is). */
  endAll(pill: string) {
    this.pills.delete(pill);
  }

  /** The pill's live sessions, oldest first. Silent ones are dropped on the way. */
  live(pill: string, now: number): LiveSession[] {
    const sessions = this.pills.get(pill);
    if (!sessions) return [];
    for (const [id, s] of sessions) {
      const silence = now - s.lastSeen;
      if (silence > SESSION_SILENCE_MS || (s.midTurn && silence > MID_TURN_SILENCE_MS)) sessions.delete(id);
    }
    if (sessions.size === 0) {
      this.pills.delete(pill);
      return [];
    }
    return [...sessions.values()].sort((a, b) => a.startedAt - b.startedAt);
  }

  /** The oldest live session: its name, folder and terminal are the card's. */
  owner(pill: string, now: number): LiveSession | null {
    return this.live(pill, now)[0] ?? null;
  }

  /** Another session of the pill is in the middle of a turn. */
  siblingMidTurn(pill: string, id: string, now: number): boolean {
    return this.live(pill, now).some((s) => s.id !== id && s.midTurn);
  }
}

export const Sessions = new PillSessions();
