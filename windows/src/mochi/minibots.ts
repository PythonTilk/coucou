// Mini Mochis (pills + compact grid) — port of MiniBotCanvasView.
// Each canvas owns a BotEngine; the island's frame loop ticks every live one.

import { BotEngine, hexToRGB } from "./engine";
import { State, type AgentTask } from "../core/state";
import { SeasonCache, outfitSelectionFor, type Outfit } from "./wardrobe";

interface MiniBot {
  canvas: HTMLCanvasElement;
  engine: BotEngine;
  /** The square the engine draws Mochi in, CSS pixels. */
  cssSize: number;
  /** Extra canvas on each side and above, for an outfit (0 when it wears none). */
  side: number;
  overhang: number;
  taskId: string;
  /** Wears its pill's outfit (the pills do; the compact grid is too small for hats). */
  dressed: boolean;
  /** Asked every frame: whether this one dances (the music pill's, MiniBotCanvasView). */
  dancing?: () => boolean;
}

/** Same proportions as the main Mochi's canvas (BOT_SIDE and BOT_OVERHANG in island.ts). */
const SIDE_RATIO = 0.25;
const OVERHANG_RATIO = 0.42;

const seasons = new SeasonCache();

function outfitOf(taskId: string): Outfit {
  return seasons.get(outfitSelectionFor(taskId, State.mainPillId, State.settings));
}

const live = new Map<HTMLCanvasElement, MiniBot>();

/**
 * Creates a mini Mochi whose **body** is `bodySize` CSS pixels across.
 *
 * The engine draws the body at 60 % of its canvas, so the canvas is
 * `bodySize / 0.6` and is centred in a `bodySize` slot, overflowing it — the
 * same thing SwiftUI does with a `.frame(width: 22/0.6)` inside a
 * `.frame(width: 22)`. Sizing the canvas itself to `bodySize` would shrink the
 * whole drawing to 60 %, which is what used to happen.
 */
export function createMiniBot(
  task: AgentTask, bodySize: number, dressed = false, dancing?: () => boolean,
): HTMLElement {
  const slot = document.createElement("span");
  slot.className = "mini";
  slot.style.width = `${bodySize}px`;
  slot.style.height = `${bodySize}px`;

  const canvas = document.createElement("canvas");
  const engineSize = bodySize / 0.6;
  // A dressed Mochi needs room for a hat's brim and tip. The body stays where
  // it was: the canvas grows around it, and is shifted up by half the room it
  // gained above.
  const side = dressed ? Math.round(engineSize * SIDE_RATIO) : 0;
  const overhang = dressed ? Math.round(engineSize * OVERHANG_RATIO) : 0;
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  canvas.width = Math.round((engineSize + side * 2) * dpr);
  canvas.height = Math.round((engineSize + overhang) * dpr);
  canvas.style.width = `${engineSize + side * 2}px`;
  canvas.style.height = `${engineSize + overhang}px`;
  canvas.style.marginTop = `${-overhang / 2}px`;
  slot.append(canvas);

  const engine = new BotEngine();
  engine.isMini = true;
  engine.bodyColor = hexToRGB(task.color);
  engine.setState(task.state, true);
  if (task.emote) engine.setPermanentEmote(task.emote);
  if (task.miniEye) {
    engine.permanentEye = task.miniEye;
    engine.eyeOverride = task.miniEye;
    engine.eyeOverrideUntil = Number.POSITIVE_INFINITY;
  }

  if (dressed) {
    engine.particleOverhang = overhang;
    engine.setOutfit(outfitOf(task.id), false);
  }

  live.set(canvas, { canvas, engine, cssSize: engineSize, side, overhang, taskId: task.id, dressed, dancing });
  return slot;
}

export function releaseMiniBot(canvas: HTMLCanvasElement) {
  live.delete(canvas);
}

/** Drops every canvas no longer in the document (views are rebuilt wholesale). */
export function pruneMiniBots() {
  for (const [canvas] of live) {
    if (!canvas.isConnected) live.delete(canvas);
  }
}

export function syncMiniBotStates(tasks: AgentTask[]) {
  for (const mb of live.values()) {
    const task = tasks.find((t) => t.id === mb.taskId);
    if (!task) continue;
    mb.engine.setState(task.state);
    mb.engine.bodyColor = hexToRGB(task.color);
    if (mb.dressed) mb.engine.setOutfit(outfitOf(mb.taskId));
  }
}

export function tickMiniBots(dt: number) {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  for (const mb of live.values()) {
    const ctx = mb.canvas.getContext("2d");
    if (!ctx) continue;
    // The canvas was sized for the scale it was created at. If the display
    // scale has changed since (another monitor, a zoom), drawing at the new
    // scale into the old size cuts the Mochi off: size it again.
    const pxW = Math.round((mb.cssSize + mb.side * 2) * dpr);
    if (mb.canvas.width !== pxW) {
      mb.canvas.width = pxW;
      mb.canvas.height = Math.round((mb.cssSize + mb.overhang) * dpr);
    }
    mb.engine.setDancing(mb.dancing?.() ?? false);
    mb.engine.update(dt);
    // Cleared in device pixels, the whole canvas. At a fractional scale (150 %)
    // the canvas is a whole number of pixels but `cssSize * dpr` is not, so
    // clearing in CSS units left the last row and column half-cleared: the glow
    // piled up there frame after frame into a thin coloured line.
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, mb.canvas.width, mb.canvas.height);
    ctx.setTransform(dpr, 0, 0, dpr, mb.side * dpr, 0);
    ctx.save();
    mb.engine.applyDance(ctx, mb.cssSize, mb.cssSize + mb.overhang);
    mb.engine.draw(ctx, mb.cssSize, mb.cssSize + mb.overhang);
    ctx.restore();
  }
}

export const miniBotCount = () => live.size;
