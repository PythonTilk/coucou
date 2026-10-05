// Focus timer view: the clock, what is being studied, and today's total.

import { h, clear, dot } from "./dom";
import { Bridge } from "../core/bridge";
import { Focus, FOCUS_PRESETS, formatClock, formatMinutes } from "../core/focus";
import type { ViewActions, ViewHost } from "./views";

const COLOURS = { idle: "#9398A1", focus: "#34D399", break: "#38BDF8" } as const;

export function buildFocus(actions: ViewActions): ViewHost {
  const who = h("div", { class: "who-row" });
  const clock = h("div", { class: "focus-clock" });
  const row = h("div", { class: "actions options" });

  const subject = h("input", {
    type: "text",
    class: "focus-subject",
    placeholder: "Subject (optional)",
    spellcheck: "false",
    maxlength: "40",
  }) as HTMLInputElement;
  // The island never holds keyboard focus on its own — it would take it from
  // whatever is being worked in. It is borrowed for this field and given back.
  subject.addEventListener("mousedown", () => void Bridge.focusWindow(true));
  subject.addEventListener("blur", () => void Bridge.focusWindow(false));
  subject.addEventListener("input", () => {
    Focus.subject = subject.value;
  });
  subject.addEventListener("keydown", (e) => {
    if ((e as KeyboardEvent).key === "Enter") subject.blur();
    e.stopPropagation(); // Escape closes the island, not this field
  });

  const card = h(
    "div",
    { class: "card wash" },
    h("div", { class: "stack", style: "padding:4px 16px 4px 116px" }, who, clock, row),
  );
  const el = h("div", { class: "view" }, card);

  // Rebuilt only when what the buttons say changes: rebuilding them between a
  // mouse-down and a mouse-up would swallow the click.
  let rowKey = "";

  const button = (label: string, kind: "primary" | "secondary", onClick: () => void) =>
    h("button", {
      class: `btn ${kind}`,
      text: label,
      onclick: () => {
        actions.blip();
        onClick();
      },
    });

  return {
    el,
    sync() {
      const phase = Focus.phase;
      card.style.setProperty(
        "--wash",
        phase === "break" ? "rgba(56,189,248,0.38)" : phase === "focus" ? "rgba(52,211,153,0.42)" : "rgba(255,255,255,0.10)",
      );

      const today = Focus.todayMinutes();
      const label =
        phase === "focus" ? (Focus.paused ? "paused" : "stay with it")
        : phase === "break" ? (Focus.paused ? "break paused" : "take a break")
        : "ready when you are";
      clear(who);
      who.append(
        dot(COLOURS[phase], 8),
        h("span", { class: "n", text: phase === "break" ? "Break" : "Focus" }),
        h("span", { text: label }),
        h("span", {
          class: "focus-today",
          text: today.total > 0 ? `Today ${formatMinutes(today.total)}` : "",
        }),
      );

      clock.textContent = formatClock(Focus.remainingMs());
      clock.style.color = phase === "idle" ? "#F5F6F8" : COLOURS[phase];

      const key = `${phase}:${Focus.paused}:${Focus.focusMin}`;
      if (key === rowKey) return;
      rowKey = key;
      clear(row);
      if (phase === "idle") {
        row.append(button("Start", "primary", () => Focus.start()));
        for (const [focusMin, breakMin] of FOCUS_PRESETS) {
          const chosen = Focus.focusMin === focusMin;
          const preset = button(`${focusMin} / ${breakMin}`, "secondary", () => Focus.setLengths(focusMin, breakMin));
          if (chosen) preset.classList.add("chosen");
          preset.title = `${focusMin} minutes of focus, ${breakMin} of break`;
          row.append(preset);
        }
        if (subject.value !== Focus.subject) subject.value = Focus.subject;
        row.append(subject);
      } else {
        row.append(
          Focus.paused
            ? button("Resume", "primary", () => Focus.resume())
            : button("Pause", "primary", () => Focus.pause()),
          button(phase === "break" ? "Skip break" : "Stop", "secondary", () => Focus.reset()),
        );
        if (Focus.subject.trim()) row.append(h("span", { class: "sub", text: Focus.subject.trim() }));
      }
    },
  };
}
