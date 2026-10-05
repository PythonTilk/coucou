// The shelf: files parked on the island. Drag one back out, or click it to put
// it on the clipboard; the × takes it off.

import { startDrag } from "@crabnebula/tauri-plugin-drag";

import { h, svg, clear } from "./dom";
import { ICONS } from "./icons";
import { Bridge, IS_TAURI } from "../core/bridge";
import { State, type ShelfItem } from "../core/state";
import type { ViewActions, ViewHost } from "./views";

/** How far the pointer must travel with the button down before it is a drag. */
const DRAG_THRESHOLD = 5;

let dragIcon: string | null = null;

/** Reads the shelf from disk — the folder is the only record of what is on it. */
export async function refreshShelf() {
  State.shelf = (await Bridge.shelfList()) ?? [];
  State.notify();
}

export async function addToShelf(path: string): Promise<void> {
  await Bridge.shelfAdd(path);
  await refreshShelf();
}

async function dragOut(item: ShelfItem) {
  if (!IS_TAURI) return;
  dragIcon ??= await Bridge.shelfDragIcon();
  // Our own file passing back over the island must not start a new drop.
  State.shelfDragging = true;
  try {
    await startDrag({ item: [item.path], icon: dragIcon });
  } finally {
    // The drop reaches the island's drag handler a moment after this resolves.
    window.setTimeout(() => {
      State.shelfDragging = false;
    }, 400);
  }
}

export function buildShelf(actions: ViewActions): ViewHost {
  const title = h("div", { class: "title", text: "Shelf" });
  const note = h("div", { class: "sub" });
  const items = h("div", { class: "shelf-items" });
  const el = h(
    "div",
    { class: "view" },
    h("div", { class: "card" }, h("div", { class: "stack shelf-body" }, title, note, items)),
  );

  const HINT = "Drag a file out, or click it to copy it.";
  const EMPTY = "Nothing here yet. Drop a file on the island and choose Keep on shelf.";
  let message: string | null = null;
  let messageTimer: number | null = null;
  let renderedKey = "";

  function say(text: string) {
    message = text;
    if (messageTimer != null) window.clearTimeout(messageTimer);
    messageTimer = window.setTimeout(() => {
      message = null;
      State.notify();
    }, 2600);
    State.notify();
  }

  function chip(item: ShelfItem): HTMLElement {
    const remove = h("button", { class: "shelf-remove", title: "Take off the shelf" }, svg(ICONS.xmark, 9));
    remove.addEventListener("mousedown", (e) => e.stopPropagation());
    remove.addEventListener("click", (e) => {
      e.stopPropagation();
      actions.blip();
      void Bridge.shelfRemove(item.path)
        .catch((err) => say(String(err).replace(/^Error:\s*/, "")))
        .then(refreshShelf);
    });

    const el = h(
      "div",
      { class: "shelf-item", title: item.name },
      svg(ICONS.doc, 12),
      h("span", { class: "shelf-name", text: item.name }),
      remove,
    );

    // One press, two meanings: let go where it started and the file is copied;
    // move away with the button down and it is dragged out.
    el.addEventListener("mousedown", (down) => {
      if (down.button !== 0) return;
      let dragging = false;
      const move = (e: MouseEvent) => {
        if (dragging) return;
        if (Math.hypot(e.clientX - down.clientX, e.clientY - down.clientY) < DRAG_THRESHOLD) return;
        dragging = true;
        release();
        void dragOut(item).catch((err) => say(String(err).replace(/^Error:\s*/, "")));
      };
      const up = () => {
        release();
        if (dragging) return;
        void Bridge.shelfCopy(item.path)
          .then(() => say(`Copied ${item.name} — paste it anywhere.`))
          .catch((err) => say(String(err).replace(/^Error:\s*/, "")));
      };
      const release = () => {
        window.removeEventListener("mousemove", move);
        window.removeEventListener("mouseup", up);
      };
      window.addEventListener("mousemove", move);
      window.addEventListener("mouseup", up);
    });
    return el;
  }

  return {
    el,
    sync() {
      note.textContent = message ?? (State.shelf.length === 0 ? EMPTY : HINT);
      // Rebuilt only when the shelf changes, so a press in progress keeps its chip.
      const key = State.shelf.map((i) => i.path).join("|");
      if (key === renderedKey) return;
      renderedKey = key;
      clear(items);
      for (const item of State.shelf) items.append(chip(item));
    },
  };
}
