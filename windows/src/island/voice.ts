// Voice (local to this build): the shortcut is pressed, a phrase is heard by
// Windows' own speech recognition, Claude answers with the user's API key, and
// the answer is shown on the island and spoken with a Windows voice
// (src-tauri/src/voice.rs).
//
// One turn at a time. Pressing the shortcut again while Mochi speaks stops him.

import { Bridge } from "../core/bridge";
import { Sound } from "../core/sound";
import { State, type AgentTask } from "../core/state";

export type VoicePhase = "listening" | "thinking" | "answer" | "error";

export interface VoiceTurn {
  phase: VoicePhase;
  heard: string;
  /** The answer, or what went wrong. */
  text: string;
}

/** What `talk` needs from the island. */
export interface VoiceHost {
  alert(view: "voice"): void;
}

/** The island in a few lines, for Claude: who is working on what, and the pills' news. */
export function describeIsland(tasks: AgentTask[], now = Date.now()): string {
  const lines: string[] = [];
  for (const task of tasks) {
    const parts = [`${task.name} (${task.isIntegration ? "service" : "agent"}): ${task.state}`];
    const step = task.steps.at(-1);
    if (step) parts.push(`last step: ${step.slice(0, 160)}`);
    if (task.finalLine) parts.push(`last words: ${task.finalLine.slice(0, 200)}`);
    lines.push(`- ${parts.join("; ")}`);
  }
  const when = new Date(now);
  const header = `Local time: ${when.toLocaleString(undefined, { weekday: "long", hour: "2-digit", minute: "2-digit" })}.`;
  return [header, lines.length ? "Pills on the island:" : "No pills on the island.", ...lines].join("\n");
}

let busy = false;

/** One spoken turn. Ignored while one is under way; stops Mochi talking if he is. */
export async function talk(host: VoiceHost): Promise<void> {
  if (stopSpeaking() || busy) return;
  busy = true;
  const show = (turn: VoiceTurn) => {
    State.voice = turn;
    State.notify();
  };
  try {
    show({ phase: "listening", heard: "", text: "" });
    host.alert("voice");
    Sound.play("blip");
    const heard = (await Bridge.voiceListen()).trim();
    if (!heard) {
      show({ phase: "error", heard: "", text: "I didn't hear anything." });
      return;
    }
    show({ phase: "thinking", heard, text: "" });
    const answer = await Bridge.voiceAnswer(heard, describeIsland(State.tasks));
    show({ phase: "answer", heard, text: answer });
    speak(answer);
  } catch (err) {
    show({ phase: "error", heard: State.voice?.heard ?? "", text: String(err).replace(/^Error:\s*/, "") });
    Sound.play("error");
  } finally {
    busy = false;
  }
}

/** Reads the answer aloud with a voice of the system's, where the page has one. */
function speak(text: string) {
  const synth = typeof window !== "undefined" ? window.speechSynthesis : undefined;
  if (!synth || !State.settings.soundEnabled) return;
  synth.cancel();
  const utterance = new SpeechSynthesisUtterance(text);
  utterance.rate = 1.05;
  synth.speak(utterance);
}

/** Stops a spoken answer. True when one was playing. */
function stopSpeaking(): boolean {
  const synth = typeof window !== "undefined" ? window.speechSynthesis : undefined;
  if (!synth?.speaking) return false;
  synth.cancel();
  return true;
}
