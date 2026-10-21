// Settings → Updates. (Local to this build, English only: upstream ships no updater.)
//
// This build is published by the fork's own workflow, and updates.rs asks that
// one address whether a newer build exists. Here it is shown, looked for by
// hand, and installed — always on a click, never on its own.

import { h, clear } from "../views/dom";
import { Bridge, onEvent, type UpdateStatus } from "../core/bridge";
import type { Settings } from "../core/state";
import { t } from "../i18n/i18n";

/** "just now", "12 min ago", "3 h ago", or the day. */
export function checkedLabel(checkedAt: number, now = Date.now()): string {
  if (!checkedAt) return "Not checked yet";
  const minutes = Math.floor((now - checkedAt * 1000) / 60_000);
  if (minutes < 1) return "Checked just now";
  if (minutes < 60) return `Checked ${minutes} min ago`;
  if (minutes < 24 * 60) return `Checked ${Math.floor(minutes / 60)} h ago`;
  return `Checked on ${new Date(checkedAt * 1000).toLocaleDateString()}`;
}

/** The one line that says where things stand. */
export function statusLine(status: UpdateStatus | null, now = Date.now()): string {
  if (!status) return "Updates are not available in this window.";
  if (status.installing) return "Installing… Coucou will close and come back.";
  if (status.available) return `Version ${status.available} is ready to install.`;
  if (status.error) return `Could not check: ${status.error}`;
  if (!status.checkedAt) return "Not checked yet.";
  return `This is the newest build. ${checkedLabel(status.checkedAt, now)}.`;
}

/** The keystore entry of the token that reads the private builds (updates.rs). */
const TOKEN_KEY = "updates-token";

/**
 * The read-only token for the private builds: saved to the OS keystore, never
 * shown again. `onSaved` looks for an update straight away.
 */
function tokenRow(onSaved: () => void): HTMLElement {
  const input = h("input", {
    type: "password", autocomplete: "off", spellcheck: "false", style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const dot = h("i", { class: "dot" });
  const save = h("button", { text: t("Save") });
  const remove = h("button", { class: "danger", text: t("Remove") });
  const said = h("div", { class: "hint" });
  let present = false;
  const draw = () => {
    dot.style.background = present ? "#22c55e" : "#f4505e";
    input.placeholder = present ? "••••••••••••  (stored)" : "github_pat_…";
    remove.style.display = present ? "" : "none";
  };
  void Bridge.secretPresent(TOKEN_KEY).then((p) => {
    present = p ?? false;
    draw();
  });
  save.addEventListener("click", async () => {
    const value = input.value.trim();
    if (!value) return;
    try {
      await Bridge.secretSet(TOKEN_KEY, value);
      input.value = "";
      present = true;
      said.textContent = "Saved in the system's credential store.";
      draw();
      onSaved();
    } catch (err) {
      said.textContent = `Could not save: ${String(err)}`;
    }
  });
  remove.addEventListener("click", async () => {
    try {
      await Bridge.secretClear(TOKEN_KEY);
      present = false;
      said.textContent = "Removed.";
      draw();
    } catch (err) {
      said.textContent = `Could not remove: ${String(err)}`;
    }
  });
  draw();
  return h(
    "div",
    { style: "display:flex;flex-direction:column;gap:6px" },
    h("div", { class: "row" }, h("label", { text: "Access token" }), dot, input, save, remove),
    h("div", {
      class: "hint",
      text: "A fine-grained GitHub token that can only read the contents of your builds repository. It is sent to GitHub and nowhere else.",
    }),
    said,
  );
}

export function updatesSection(
  settings: Settings,
  toggle: (on: boolean, onChange: (v: boolean) => void) => HTMLElement,
  save: () => void,
): HTMLElement {
  const dot = h("i", { class: "dot" });
  const line = h("div", { class: "hint" });
  const notes = h("div", { class: "hint update-notes" });
  const buttons = h("div", { class: "row" });
  let status: UpdateStatus | null = null;
  let busy = false;

  function draw() {
    const waiting = !!status?.available;
    dot.style.background = waiting ? "#f5b544" : "#22c55e";
    dot.style.display = status ? "" : "none";
    line.textContent = busy && !status?.installing ? "Checking…" : statusLine(status);
    notes.textContent = waiting ? (status?.notes ?? "") : "";
    notes.style.display = notes.textContent ? "" : "none";

    clear(buttons);
    const check = h("button", { text: t("Check now") }) as HTMLButtonElement;
    check.disabled = busy || !!status?.installing;
    check.addEventListener("click", () => void look());
    buttons.append(h("label", { text: status ? `Version ${status.current}` : "Version" }), check);
    if (waiting) {
      const install = h("button", { class: "primary", text: `Install ${status!.available} and restart` }) as HTMLButtonElement;
      install.disabled = busy || status!.installing;
      install.addEventListener("click", () => void install_());
      buttons.append(install);
    }
  }

  async function look() {
    busy = true;
    draw();
    status = (await Bridge.updateCheck()) ?? status;
    busy = false;
    draw();
  }

  async function install_() {
    busy = true;
    if (status) status = { ...status, installing: true };
    draw();
    try {
      await Bridge.updateInstall();
    } catch (err) {
      // The installer did not start: say why, and let it be tried again.
      const fresh = await Bridge.updateStatus();
      status = fresh ?? (status ? { ...status, installing: false, error: String(err) } : null);
    }
    busy = false;
    draw();
  }

  void Bridge.updateStatus().then((s) => {
    status = s;
    draw();
  });
  void onEvent<UpdateStatus>("update-status", (s) => {
    status = s;
    draw();
  });

  const section = h(
    "section",
    {},
    h("h2", {}, dot, h("span", { text: t("Updates") })),
    h("div", {
      class: "hint",
      text: "This build comes from your own fork. Its builds are kept in a private repository of yours; Coucou asks it for the newest one and installs it when you say so. Windows asks for administrator rights at each install.",
    }),
    buttons,
    line,
    notes,
    tokenRow(() => void look()),
    h("div", { class: "row" },
      h("label", { text: "Check automatically" }),
      toggle(settings.checkForUpdates, (v) => {
        settings.checkForUpdates = v;
        save();
      }),
      h("span", { class: "hint", text: "a few times a day" }),
    ),
  );
  // The tray's update entry opens Settings: unfold this section and bring it into view.
  void onEvent<null>("show-updates", () => {
    if (section.classList.contains("folded")) section.querySelector<HTMLElement>(":scope > h2")?.click();
    section.scrollIntoView({ block: "start", behavior: "smooth" });
  });
  draw();
  return section;
}
