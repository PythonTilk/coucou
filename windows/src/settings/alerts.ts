// Settings → Phone alerts (local to this build): the ntfy topic Coucou posts a
// short notice to when an agent needs you and you are away from the computer
// (src-tauri/src/alerts.rs).

import { h } from "../views/dom";
import { Bridge } from "../core/bridge";
import { statusDot } from "./parts";
import { t } from "../i18n/i18n";

const KEY = "ntfy-topic-url";

export function alertsSection(): HTMLElement {
  const input = h("input", {
    type: "password", autocomplete: "off", spellcheck: "false", style: "flex:1 1 auto;min-width:0",
  }) as HTMLInputElement;
  const dot = statusDot(false);
  const save = h("button", { text: t("Save") });
  const remove = h("button", { class: "danger", text: t("Remove") });
  const test = h("button", { text: "Send a test" });
  const said = h("div", { class: "hint" });
  let present = false;

  const draw = () => {
    dot.style.background = present ? "#22c55e" : "#f4505e";
    input.placeholder = present ? "••••••••••••  (stored)" : "https://ntfy.sh/your-secret-topic";
    remove.style.display = present ? "" : "none";
    test.style.display = present ? "" : "none";
  };
  void Bridge.secretPresent(KEY).then((p) => {
    present = p ?? false;
    draw();
  });

  save.addEventListener("click", async () => {
    const value = input.value.trim();
    if (!/^https:\/\/[^\s/?#@]+\.[^\s/?#@]+\/[^\s?#]+$/.test(value)) {
      said.textContent = "That is not an https link to a topic, like https://ntfy.sh/your-secret-topic.";
      return;
    }
    try {
      await Bridge.secretSet(KEY, value);
      input.value = "";
      present = true;
      said.textContent = "Saved in the system's credential store. Send a test to check it reaches your phone.";
      draw();
    } catch (err) {
      said.textContent = `Could not save: ${String(err)}`;
    }
  });
  remove.addEventListener("click", async () => {
    try {
      await Bridge.secretClear(KEY);
      present = false;
      said.textContent = "Removed. No alerts are sent.";
      draw();
    } catch (err) {
      said.textContent = `Could not remove: ${String(err)}`;
    }
  });
  test.addEventListener("click", async () => {
    said.textContent = t("Sending…");
    try {
      const sent = await Bridge.phoneAlertTest();
      said.textContent = sent ? "Sent. It should be on your phone in a moment." : "Nothing was sent.";
    } catch (err) {
      said.textContent = String(err).replace(/^Error:\s*/, "");
    }
  });

  draw();
  return h(
    "section",
    {},
    h("h2", {}, h("span", { text: "Phone alerts" })),
    h("div", {
      class: "hint",
      text: "When an agent needs permission, asks a question, finishes or stops with an error while nobody is at this computer, Coucou posts a short notice to your ntfy topic: who, and what kind of thing, never the command or the question. Install the ntfy app on your phone and subscribe to the same topic. Tapping the notice opens Islet.",
    }),
    h("div", { class: "row" }, h("label", { text: "ntfy topic" }), dot, input, save, remove),
    h("div", { class: "row" }, test),
    h("div", {
      class: "hint",
      text: "Pick a topic name nobody would guess: anyone who knows it can read the notices. The address is kept in the system's credential store.",
    }),
    said,
  );
}
