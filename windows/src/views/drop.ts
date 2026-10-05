// The drop menu: what the island shows while a file is being carried. Two
// halves — the chat on the left, the shelf on the right — and the file goes to
// whichever it is let go over. It is never shown without a file in the air.

import { h, svg } from "./dom";
import { ICONS } from "./icons";
import { dashedFrame } from "./upload";
import { State, type DropTarget } from "../core/state";
import type { ViewHost } from "./views";

function half(target: DropTarget, icon: string, title: string, sub: string): HTMLElement {
  return h(
    "div",
    { class: "card drop-card drop-half", "data-target": target },
    dashedFrame(),
    h(
      "div",
      { class: "drop-half-body" },
      svg(icon, 18),
      h("div", { class: "drop-title", text: title }),
      h("div", { class: "drop-sub", text: sub }),
    ),
  );
}

export function buildDrop(): ViewHost {
  const chat = half("chat", ICONS.bubble, "Ask about it", "Summarise, explain, quiz");
  const shelf = half("shelf", ICONS.stack, "Keep on the shelf", "Take it away later");
  const el = h("div", { class: "view drop-view" }, chat, shelf);

  return {
    el,
    sync() {
      chat.classList.toggle("over", State.dropTarget === "chat");
      shelf.classList.toggle("over", State.dropTarget === "shelf");
    },
  };
}
