// The voice card (island/voice.ts): what was heard, and Mochi's answer.

import { h } from "./dom";
import { State } from "../core/state";
import type { ViewHost } from "./views";
import { t } from "../i18n/i18n";

const COLOURS = { listening: "#38BDF8", thinking: "#8B5CF6", answer: "#34D399", error: "#F4505E" } as const;
/** What the card is called in each phase. */
const label_ = (phase: keyof typeof COLOURS): string =>
  phase === "listening" ? t("Listening…") : phase === "thinking" ? "Thinking…" : phase === "answer" ? "Mochi" : t("Voice");

export function buildVoice(): ViewHost {
  const label = h("div", { class: "voice-label" });
  const heard = h("div", { class: "voice-heard" });
  const text = h("div", { class: "voice-text" });
  const card = h(
    "div",
    { class: "card wash" },
    h("div", { class: "stack", style: "padding:4px 16px 4px 116px" }, label, heard, text),
  );
  return {
    el: h("div", { class: "view" }, card),
    sync() {
      const turn = State.voice;
      const phase = turn?.phase ?? "listening";
      card.style.setProperty("--wash", COLOURS[phase]);
      label.textContent = label_(phase);
      label.style.color = COLOURS[phase];
      heard.textContent = turn?.heard ? `“${turn.heard}”` : phase === "listening" ? "Say something." : "";
      text.textContent = turn?.text ?? "";
      text.classList.toggle("error", phase === "error");
    },
  };
}
