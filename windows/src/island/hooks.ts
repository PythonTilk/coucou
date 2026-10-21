// Hook events from Claude Code and every other agent → island state.
// Port of HookServer.processEvent / processPermissionRequest from the macOS app.
// Difference from macOS: no terminal filter. On Windows the hook fires from any
// terminal (Windows Terminal, VS Code, PowerShell…) and all of them are handled.
// The relay has already mapped every agent's events and fields onto Claude
// Code's (hook/src/normalize.rs), so one handler serves them all.

import { Bridge, onEvent } from "../core/bridge";
import { buildFileDiff, fileName, makeDiffStep, toOneLine } from "../core/diff";
import { Sound } from "../core/sound";
import { State, type AskedQuestion } from "../core/state";
import { pillDefinition } from "../core/pills";
import { APPROVAL_AGENTS, CLAUDE_CODE_TAGS, agentColor, agentName, validateAgent } from "./agents";
import type { Island } from "./island";
import { parseClaudePlan, restorePlanUsage } from "../core/plan";
import { setClaudePlanUsage, storedClaudePlanUsage } from "../views/usage";
import { N_, t } from "../i18n/i18n";
import { Sessions } from "./sessions";

const CLAUDE_ID = "integration_claude";
const CURSOR_ID = "agent_cursor";

/**
 * How many requests may wait behind the card that is up. Past that the newest
 * goes back to its terminal (five in all, as on the Mac).
 */
export const MAX_WAITING = 4;

/**
 * Each request's own 110 s: Coucou answers within 108 s or not at all, after
 * which the terminal has taken over and a card for it would be lying.
 */
const requestTimers = new Map<string, number>();

function disarm(requestId: string) {
  const timer = requestTimers.get(requestId);
  if (timer == null) return;
  window.clearTimeout(timer);
  requestTimers.delete(requestId);
}

/**
 * The next request in line takes the card. False when nobody is waiting. Called
 * wherever a card has just gone: answered, withdrawn, or timed out.
 */
export function promoteNextApproval(island: Island): boolean {
  if (State.pendingApproval) return true;
  const next = State.approvalQueue.shift();
  if (!next) return false;
  const view = next.questions ? "question" : "approval";
  State.beginApproval(next);
  State.updateTask(next.pillId, view);
  Sound.play(view);
  island.alert(view);
  State.notify();
  return true;
}

/** Takes the approval or question card down and gives the island back, or to the next request. */
export function dropPendingCard(island: Island): void {
  const pending = State.pendingApproval;
  if (!pending) return;
  disarm(pending.requestId);
  State.endApproval();
  if (promoteNextApproval(island)) return;
  island.dropPin();
  if (State.view === "approval" || State.view === "question") {
    island.setView(State.defaultView());
  }
  State.notify();
}

/** A request is moot: its card goes, or it leaves the line. The relay is told when `decline`. */
function withdraw(island: Island, requestId: string, decline: boolean) {
  disarm(requestId);
  if (State.pendingApproval?.requestId === requestId) {
    if (decline && requestId) void Bridge.approvalDecline(requestId);
    dropPendingCard(island);
    return;
  }
  const at = State.approvalQueue.findIndex((r) => r.requestId === requestId);
  if (at < 0) return;
  State.approvalQueue.splice(at, 1);
  if (decline && requestId) void Bridge.approvalDecline(requestId);
  State.notify();
}

/**
 * A notice to the phone through ntfy (alerts.rs, local to this build): who,
 * and what kind of thing, never the command or the question. Rust sends it
 * only when nobody is at the computer and alerts are set up.
 */
function alertPhone(kind: "approval" | "question" | "finished" | "error", pillId: string) {
  const name = State.tasks.find((t) => t.id === pillId)?.name ?? "";
  void Bridge.phoneAlert(kind, name);
}

/** Forgets the sessions and the waiting requests (tests). */
export function resetHookState() {
  for (const timer of requestTimers.values()) window.clearTimeout(timer);
  requestTimers.clear();
  State.approvalQueue = [];
  Sessions.clear();
}

/** The return to idle that Stop arms, per pill, so the next turn can cancel it. */
const stopTimers = new Map<string, number>();

function cancelStopTimer(id: string): boolean {
  const timer = stopTimers.get(id);
  if (timer == null) return false;
  window.clearTimeout(timer);
  stopTimers.delete(id);
  return true;
}

/** Events after which a pending permission request of the same session is moot. */
const TURN_OVER = new Set(["Stop", "StopFailure", "UserPromptSubmit", "SessionEnd", "Interrupt"]);

/** Events only a running session sends: these may add it to its pill's sessions. */
const SESSION_OPENS = new Set(["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest"]);
/** A turn is under way after these, and over after those. */
const TURN_STARTS = new Set(["UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "PermissionRequest"]);
const TURN_ENDS = new Set(["Stop", "StopFailure", "Interrupt"]);

interface HookPayload {
  hook_event_name?: string;
  request_id?: string;
  session_id?: string;
  cwd?: string;
  message?: string;
  /** UserPromptSubmit carries `prompt`; `message` belongs to Notification/Stop. */
  prompt?: string;
  /** Stop: the turn's final answer (Markdown) — Claude Code's, Hermes's, Codex's. */
  last_assistant_message?: string;
  tool_name?: string;
  tool_input?: Record<string, unknown>;
  /** Set by the relay when an edit was too big to forward whole (> 256 KB). */
  coucou_diff_truncated?: boolean;
  /** Optional agent tag: lowercase, digits and hyphens, ≤ 24 chars. */
  coucou_agent?: string;
  /** Hermes: where the session runs (telegram, discord…; "cli" in a terminal). */
  platform?: string;
  /** "cursor" when Claude Code runs in Cursor's terminal (set by the relay). */
  term_editor?: string;
  /** StatusLine (the plan usage relay): Claude Code's 5-hour and weekly limits. */
  rate_limits?: unknown;
}

const PROJECT_ALIASES: Record<string, string> = {
  "notch-buddy": "Notch Buddy",
  notchbuddy: "Notch Buddy",
  notch_buddy: "Notch Buddy",
};

function aliasProjectName(name: string): string {
  return PROJECT_ALIASES[name.toLowerCase()] ?? name;
}

function lastPathComponent(p: string): string {
  const cleaned = p.replace(/[\\/]+$/, "");
  const idx = Math.max(cleaned.lastIndexOf("\\"), cleaned.lastIndexOf("/"));
  return idx >= 0 ? cleaned.slice(idx + 1) : cleaned;
}

/**
 * localizedStep() — same labels as the macOS app, in English and shown in the
 * interface language (src/i18n) when the step is recorded.
 */
const TOOL_LABELS: Record<string, string> = {
  Bash: N_("Runs"),
  Read: N_("Reads"),
  Write: N_("Writes"),
  Edit: N_("Edits"),
  Glob: N_("Searches"),
  Grep: N_("Searches"),
  WebSearch: N_("Searches the web"),
  WebFetch: N_("Fetches"),
  TodoWrite: N_("Tasks"),
  Task: N_("Agent"),
  LS: N_("Lists"),
  MultiEdit: N_("Edits"),
  NotebookEdit: N_("Notebook"),
  PowerShell: N_("Runs"),
  // Antigravity's tools (#298).
  run_command: N_("Runs"),
  view_file: N_("Reads"),
  write_to_file: N_("Writes"),
  replace_file_content: N_("Edits"),
  read_url_content: N_("Fetches"),
  search_web: N_("Searches the web"),
  ask_question: N_("Needs input"),
};

function stepLabel(tool: string, input: Record<string, unknown>): string {
  const labelTool = tool === "ask_question" || tool.endsWith(":ask_question") ? "ask_question" : tool;
  const label = TOOL_LABELS[labelTool] ? t(TOOL_LABELS[labelTool]) : tool;
  const str = (k: string) => (typeof input[k] === "string" ? (input[k] as string) : null);
  const cmd = str("command");
  if (cmd) return `${label} · ${cmd.slice(0, 40)}`;
  const path = str("path");
  if (path) return `${label} · ${lastPathComponent(path)}`;
  const file = str("file_path");
  if (file) return `${label} · ${lastPathComponent(file)}`;
  const query = str("query");
  if (query) return `${label} · ${query.slice(0, 40)}`;
  if (
    (tool === "ask_question" || tool.endsWith(":ask_question")) &&
    Array.isArray(input.questions) &&
    input.questions.length > 0
  ) {
    const firstQ = input.questions[0];
    if (typeof firstQ === "object" && firstQ !== null && typeof (firstQ as Record<string, unknown>).question === "string") {
      const q = ((firstQ as Record<string, unknown>).question as string).trim();
      if (q) return `${label} · ${q.slice(0, 40)}`;
    }
  }
  return label;
}

/**
 * What the Allow button actually authorises. Approving "Write" tells you nothing
 * — approving `Write · C:\…\.env` tells you everything, and the difference is
 * the whole point of approving from the island rather than blind.
 *
 * Ordered by how specific the field is, so an unfamiliar tool still shows
 * whatever identifying string it carries instead of falling back to its name.
 */
const APPROVAL_FIELDS = [
  "command", // Bash, PowerShell
  "file_path", // Write, Edit, MultiEdit, NotebookEdit
  "path", // Read, LS
  "url", // WebFetch
  "query", // WebSearch
  "pattern", // Glob, Grep
  "prompt", // Task
] as const;

function approvalTarget(tool: string, input: Record<string, unknown>): string {
  for (const field of APPROVAL_FIELDS) {
    const value = input[field];
    if (typeof value === "string" && value.trim()) {
      return `${tool} · ${value.trim()}`;
    }
  }
  return tool;
}

/**
 * The questions of an AskUserQuestion call, if the island can show all of them
 * as options to pick from. Anything it cannot is left to the terminal.
 */
function askedQuestions(tool: string, input: Record<string, unknown>): AskedQuestion[] | null {
  if (tool !== "AskUserQuestion" || !Array.isArray(input.questions)) return null;
  const out: AskedQuestion[] = [];
  for (const raw of input.questions as Record<string, unknown>[]) {
    const question = typeof raw?.question === "string" ? raw.question : "";
    const options = (Array.isArray(raw?.options) ? (raw.options as Record<string, unknown>[]) : [])
      .filter((o) => typeof o?.label === "string" && o.label)
      .map((o) => ({
        label: o.label as string,
        description: typeof o.description === "string" ? o.description : "",
      }));
    // A question cut short by the relay would be answered under the wrong text.
    if (!question || question.endsWith("…") || options.length < 2) return null;
    out.push({ question, options, multiSelect: raw.multiSelect === true });
  }
  return out.length > 0 ? out : null;
}

/** The Claude Code session's pill, named after its project for the session. */
function upsert(id: string, projectName: string, cwd: string, sessionId: string) {
  const t = State.upsertWorkspacePill(id, projectName, cwd);
  if (t && sessionId) t.sessionId = sessionId;
}

/** The session is over: the pill goes back as it was, or away if it was only there for it. */
function clearSession(id: string) {
  const t = State.tasks.find((x) => x.id === id);
  if (!t) return;
  if (!State.isKept(id)) {
    State.removeTask(id);
    return;
  }
  t.state = "idle";
  t.steps = [];
  t.stepIndex = 0;
  delete t.stepSeq;
  t.name = pillDefinition(id)?.name ?? t.name;
  t.pillBadge = null;
  t.sessionId = null;
  t.finalLine = null;
}

/** The final message stays on the card until the next turn starts. */
function clearFinalLine(id: string) {
  const t = State.tasks.find((x) => x.id === id);
  if (t) t.finalLine = null;
}

/**
 * PostToolUse of Edit / MultiEdit / Write → a diff stored for the pill and a
 * ticker step with its +N −M counts. Nothing is kept for a pill that does not
 * exist, so a stray event cannot grow memory. An edit the relay had to cut
 * would give wrong counts: the PreToolUse step ("Edits · file") stands alone.
 */
function recordDiff(agentId: string, payload: HookPayload) {
  if (payload.coucou_diff_truncated) return;
  if (!State.tasks.some((t) => t.id === agentId)) return;
  const diff = buildFileDiff(payload.tool_name ?? "", payload.tool_input ?? {});
  if (!diff) return;
  const id = State.appendSessionDiff(agentId, diff);
  State.appendStep(agentId, makeDiffStep(fileName(diff.path), diff.added, diff.removed, id));
}

export function registerHookHandlers(island: Island) {
  // The last plan numbers seen survive a restart, as on the Mac.
  State.planUsage ??= restorePlanUsage(storedClaudePlanUsage());
  void onEvent<HookPayload>("hook", (payload) => handleHook(island, payload));
}

function handleHook(island: Island, payload: HookPayload) {
  // Account-wide numbers from the status line relay, not part of any session:
  // keep the latest, nothing else (no reveal, no sound), paused or not.
  if (payload.hook_event_name === "StatusLine") {
    const usage = parseClaudePlan(payload.rate_limits);
    if (usage) setClaudePlanUsage(usage);
    return;
  }

  if (State.paused) {
    // Silence here used to cost Claude Code nearly two minutes: the relay waited
    // for a decision from an island that had already decided not to look. Say so,
    // and the terminal takes the question immediately.
    if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
    return;
  }

  const name = payload.hook_event_name ?? "";
  const cwd = payload.cwd ?? "";
  const raw = lastPathComponent(cwd);
  const projectName = aliasProjectName(raw || "Session");

  // Route to the right pill. Valid coucou_agent → dynamic "agent_<name>" pill.
  // "claude" is reserved; absent or invalid → Claude Code's own pill: Cursor's
  // when it runs in Cursor's terminal (Mac #120), VS Code's otherwise.
  const validAgent = validateAgent(payload.coucou_agent);
  const workspaceId = payload.term_editor === "cursor" ? CURSOR_ID : CLAUDE_ID;
  const agentId = validAgent ? `agent_${validAgent}` : workspaceId;
  const isExternalAgent = validAgent !== null;
  const sessionId = payload.session_id ?? "";

  // Who is alive behind this pill (sessions.ts). Only the events a running
  // session sends may add one; the rest refresh it.
  const now = Date.now();
  if (name !== "SessionEnd") {
    Sessions.seen(agentId, sessionId, now, { projectName, cwd }, SESSION_OPENS.has(name));
    if (TURN_STARTS.has(name)) Sessions.turn(agentId, sessionId, true);
    else if (TURN_ENDS.has(name)) Sessions.turn(agentId, sessionId, false);
  }
  /** A sibling session of the same pill is still in the middle of its turn. */
  const siblingBusy = () => Sessions.siblingMidTurn(agentId, sessionId, now);
  /**
   * The pill's state, unless a card of another session is waiting on it: a
   * sibling at work must not take "needs permission" off the pill.
   */
  const setState = (state: Parameters<typeof State.updateTask>[1]) => {
    const card = State.pendingApproval;
    if (card && card.pillId === agentId && card.sessionId !== sessionId) return;
    State.updateTask(agentId, state);
  };

  /** Alerts force the island open; work events only reveal the compact island. */
  const surface = (view: Parameters<Island["alert"]>[0], isAlert: boolean) => {
    if (State.mode === "expanded") {
      if (isAlert) island.setView(view);
    } else if (isAlert) {
      island.alert(view);
    } else if (State.mode === "hidden") {
      island.reveal();
    }
  };

  /** Ensure the agent pill exists (no-op for Claude Code). */
  const ensurePill = () => {
    // The oldest live session owns what the card says: its project, its folder
    // and its terminal. They pass to the next one when it ends.
    const owner = Sessions.owner(agentId, now);
    const own = owner ?? { projectName, cwd, id: sessionId };
    if (isExternalAgent) {
      State.upsertExternalAgent(agentId, agentName(validAgent!), agentColor(validAgent!));
      const t = State.tasks.find((x) => x.id === agentId);
      if (t && own.cwd) t.sessionCwd = own.cwd;
      if (t && own.id) t.sessionId = own.id;
    } else {
      upsert(agentId, own.projectName, own.cwd, own.id);
    }
  };

  /**
   * Called where a handler is about to replace `finished` with a newer state:
   * the timer Stop armed would otherwise put the pill back to idle over it. The
   * badge that timer was going to clear goes now.
   */
  const supersedeStop = () => {
    if (cancelStopTimer(agentId)) State.setPillBadge(agentId, null);
  };

  // The turn that asked for a permission is over — answered in the terminal,
  // interrupted, or a new prompt — so the card would be lying. It goes, and the
  // relay is released without a decision. Same rule as the Mac.
  if (TURN_OVER.has(name)) {
    const moot = [State.pendingApproval, ...State.approvalQueue].filter(
      (r): r is NonNullable<typeof r> => !!r && r.pillId === agentId && r.sessionId === sessionId,
    );
    // Those in line first, so the card that goes is not replaced by one of them.
    for (const r of moot.reverse()) withdraw(island, r.requestId, true);
  }

  // Read after the card above is dropped: its pill may have handed the front
  // back to the pill you were on.
  const focused = State.focusId === agentId;

  switch (name) {
    case "SessionStart":
      ensurePill();
      clearFinalLine(agentId);
      // Hermes through its gateway says where the session comes from.
      if (validAgent === "hermes" && payload.platform && payload.platform !== "cli") {
        State.appendStep(agentId, payload.platform.charAt(0).toUpperCase() + payload.platform.slice(1));
      }
      surface("overview", false);
      Sound.play("work");
      break;

    case "UserPromptSubmit": {
      ensurePill();
      supersedeStop();
      clearFinalLine(agentId);
      setState("thinking");
      // The field is `prompt`; reading `message` meant this step was always blank.
      const asked = payload.prompt ?? payload.message;
      if (asked) State.appendStep(agentId, asked.slice(0, 60));
      surface("overview", false);
      break;
    }

    case "PreToolUse": {
      ensurePill();
      supersedeStop();
      clearFinalLine(agentId);
      const tool = payload.tool_name ?? "Tool";
      const isQuestion = tool === "ask_question" || tool.endsWith(":ask_question");
      if (isQuestion) {
        setState("question");
        State.appendStep(agentId, stepLabel(tool, payload.tool_input ?? {}));
        Sound.play("question");
        surface("overview", true);
      } else {
        setState("working");
        State.appendStep(agentId, stepLabel(tool, payload.tool_input ?? {}));
        surface("overview", false);
      }
      break;
    }

    case "PostToolUse":
      supersedeStop();
      // The question was answered in the terminal: the card would be lying.
      if (
        payload.tool_name === "AskUserQuestion" &&
        State.pendingApproval?.questions &&
        State.pendingApproval.sessionId === (payload.session_id ?? "")
      ) {
        dropPendingCard(island);
      }
      setState("working");
      recordDiff(agentId, payload);
      break;

    case "PostToolUseFailure":
      supersedeStop();
      setState("working");
      State.appendStep(agentId, t("⚠ failed"));
      break;

    case "Notification": {
      const message = payload.message ?? "";
      const lower = message.toLowerCase();
      if (lower.includes("rate limit") || lower.includes("limite d")) {
        supersedeStop();
        setState("ratelimit");
        Sound.play("rate");
      } else if (message.endsWith("?")) {
        supersedeStop();
        setState("question");
        State.appendStep(agentId, message);
      }
      break;
    }

    case "Stop": {
      // A sibling still at work keeps the pill: this turn's last words go in the
      // ticker, and the pill finishes when the last of them does.
      if (siblingBusy()) {
        const said = toOneLine(payload.last_assistant_message ?? payload.message ?? "");
        if (said) State.appendStep(agentId, said);
        break;
      }
      State.updateTask(agentId, "finished");
      alertPhone("finished", agentId);
      // Claude Code puts the turn's answer in the Stop payload itself, so there is
      // no transcript to read (the relay does not even forward its path). Other
      // agents report their last words the same way (Hermes, Codex) or as
      // `message`.
      const finalText = toOneLine(payload.last_assistant_message ?? payload.message ?? "");
      if (finalText) {
        State.appendStep(agentId, finalText);
        const t = State.tasks.find((x) => x.id === agentId);
        if (t) t.finalLine = finalText;
      }
      Sound.play("finish");
      // A card waiting for an answer is never covered by another alert.
      if (focused && !State.pendingApproval) surface("finished", true);
      else State.setPillBadge(agentId, "finished");
      cancelStopTimer(agentId);
      stopTimers.set(
        agentId,
        window.setTimeout(() => {
          stopTimers.delete(agentId);
          if (isExternalAgent) {
            State.removeTask(agentId);
          } else {
            State.updateTask(agentId, "idle");
            State.setPillBadge(agentId, null);
          }
        }, 5200),
      );
      break;
    }

    case "Interrupt":
      if (siblingBusy()) break;
      // Codex: the user stopped the turn. Back to idle, nothing to celebrate.
      supersedeStop();
      State.updateTask(agentId, "idle");
      State.setPillBadge(agentId, null);
      break;

    case "StopFailure":
      if (siblingBusy()) {
        State.appendStep(agentId, t("⚠ failed"));
        break;
      }
      supersedeStop();
      State.updateTask(agentId, "error");
      alertPhone("error", agentId);
      Sound.play("error");
      if (focused && !State.pendingApproval) surface("error", true);
      else State.setPillBadge(agentId, "error");
      break;

    case "SessionEnd":
      // An end that does not say whose it is ends them all, as it always did.
      if (sessionId) Sessions.end(agentId, sessionId);
      else Sessions.endAll(agentId);
      // Only the last live session takes the card and its diffs down. The name,
      // folder and terminal pass to the oldest one left.
      if (Sessions.live(agentId, now).length > 0) {
        ensurePill();
        break;
      }
      // Nothing left for the timer to do, and it must not outlive the session: a
      // pill recreated within 5.2 s would be removed by it.
      cancelStopTimer(agentId);
      State.clearSessionDiffs(agentId);
      if (isExternalAgent) {
        State.removeTask(agentId);
      } else {
        State.updateTask(agentId, "idle");
        clearSession(agentId);
      }
      break;

    case "SubagentStart":
      State.appendStep(agentId, t("+ subagent"));
      break;

    case "SubagentStop":
      State.appendStep(agentId, t("• subagent done"));
      break;

    case "PermissionRequest": {
      // Only Claude Code and the agents the relay can answer for (Codex, Copilot
      // CLI, Muse Code — same as the Mac) get a card. Anyone else's request is
      // declined at once, so the agent asks in its own terminal.
      // Claude Code under the desktop app's pill is still Claude Code.
      const isClaudeCode = !isExternalAgent || CLAUDE_CODE_TAGS.has(validAgent!);
      if (!isClaudeCode && !APPROVAL_AGENTS.has(validAgent!)) {
        if (payload.request_id) void Bridge.approvalDecline(payload.request_id);
        break;
      }

      const requestId = payload.request_id ?? "";
      const waiting = State.pendingApproval != null && State.pendingApproval.requestId !== requestId;
      // One card, one request: a second one never replaces the first. It waits
      // in line behind it — unless the line is full, or it has no id to be told
      // apart by, in which case it goes straight back to its terminal.
      if (waiting && (!requestId || State.approvalQueue.length >= MAX_WAITING)) {
        if (requestId) void Bridge.approvalDecline(requestId);
        break;
      }
      ensurePill();
      supersedeStop();
      disarm(requestId);
      const tool = payload.tool_name ?? "Tool";
      const input = payload.tool_input ?? {};
      // Claude Code asking a question is not a permission to grant: the island
      // shows the options and sends back the one that was picked. Only Claude
      // Code asks questions this way.
      const questions = isClaudeCode ? askedQuestions(tool, input) : null;
      const view = questions ? "question" : "approval";
      const request = {
        requestId,
        sessionId,
        pillId: agentId,
        tool,
        command: approvalTarget(tool, input),
        ...(questions ? { questions } : {}),
        // Named only when a pill is shared: its card says the owner's project.
        ...(cwd && Sessions.live(agentId, now).length > 1 ? { projectName } : {}),
      };
      // Each request gets its own 110 s, on the card or in the line.
      const arm = () =>
        requestTimers.set(
          requestId,
          window.setTimeout(() => {
            requestTimers.delete(requestId);
            withdraw(island, requestId, false);
          }, 110_000),
        );
      if (waiting) {
        if (!State.approvalQueue.some((r) => r.requestId === requestId)) State.approvalQueue.push(request);
        // The relay is told it has been taken, or it gives up after 800 ms.
        void Bridge.approvalAck(requestId);
        arm();
        alertPhone(view, agentId);
        break;
      }
      // The card always comes up, even over another pill or an island that is
      // already open: its pill comes to the front, and the one you were on
      // comes back once you answer (Mac #117, #120).
      State.beginApproval(request);
      // The relay's short ack window closes in 800 ms; everything below this
      // line is synchronous, so the card really is up by the time it lands.
      if (requestId) void Bridge.approvalAck(requestId);
      State.updateTask(agentId, view);
      Sound.play(view);
      // Any agent's card (Claude Code, Codex, Copilot CLI, Muse Code) comes up
      // the same way: beginApproval brought its pill to the front.
      island.alert(view);
      arm();
      alertPhone(view, agentId);
      break;
    }

    default:
      break;
  }
  State.notify();
}
