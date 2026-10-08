// Settings sections that fold, gathered under a few headings.
//
// A section is left as its builder made it — several redraw themselves — and is
// folded by a class alone: everything in it but its title is hidden. Which ones
// are open is remembered, per section, on this machine.

import { h } from "../views/dom";

export interface SettingsGroup {
  title: string;
  /** Each section with the name its fold is remembered under. */
  sections: [id: string, el: HTMLElement][];
}

const STORE_KEY = "coucou.settings.open";

function readOpen(): Set<string> {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(STORE_KEY) ?? "[]");
    return new Set(Array.isArray(saved) ? saved.filter((v): v is string => typeof v === "string") : []);
  } catch {
    return new Set();
  }
}

function writeOpen(open: Set<string>) {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify([...open]));
  } catch {
    // No storage: the folds simply are not remembered.
  }
}

/** Makes `section` fold on its title. Folded unless it was left open. */
function fold(id: string, section: HTMLElement, open: Set<string>) {
  const title = section.querySelector<HTMLElement>(":scope > h2");
  if (!title) return;
  const set = (isOpen: boolean) => {
    section.classList.toggle("folded", !isOpen);
    title.setAttribute("aria-expanded", String(isOpen));
  };
  const flip = () => {
    const isOpen = section.classList.contains("folded");
    if (isOpen) open.add(id);
    else open.delete(id);
    writeOpen(open);
    set(isOpen);
  };
  section.classList.add("foldable");
  title.setAttribute("role", "button");
  title.tabIndex = 0;
  title.addEventListener("click", flip);
  title.addEventListener("keydown", (e) => {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    flip();
  });
  set(open.has(id));
}

/** The groups as elements: a heading, then its sections, each foldable. */
export function foldedGroups(groups: SettingsGroup[]): HTMLElement[] {
  const open = readOpen();
  return groups.map((group) => {
    for (const [id, el] of group.sections) fold(id, el, open);
    return h(
      "div",
      { class: "settings-group" },
      h("h3", { class: "settings-group-title", text: group.title }),
      ...group.sections.map(([, el]) => el),
    );
  });
}
