#!/usr/bin/env node
// A local "OpenAI-compatible server" that answers with your Claude subscription.
// (Local to this fork.)
//
// Coucou's chat can talk to a local server such as Ollama or LM Studio:
// GET /v1/models and a streamed POST /v1/chat/completions. This script speaks
// that, and answers each request by running Claude Code once (`claude -p`),
// signed in with your Claude plan. No API key, nothing besides Claude Code.
//
//   node claude-subscription-proxy.mjs            listens on 127.0.0.1:11435
//   PORT=11500 node claude-subscription-proxy.mjs
//
// In Coucou: Settings → Chat → local server (Ollama / LM Studio / custom),
// address http://127.0.0.1:11435, then pick sonnet, opus or haiku.
//
// Claude Code runs with no tools, no MCP servers and none of your settings:
// it can only write an answer. Only this computer can reach the server.

import { createServer } from "node:http";
import { spawn } from "node:child_process";

const HOST = "127.0.0.1";
const PORT = Number(process.env.PORT) || 11435;
const CLAUDE = process.env.CLAUDE_BIN || "claude";
const MODELS = ["sonnet", "opus", "haiku"];
const MAX_BODY = 20 * 1024 * 1024;

const log = (...a) => console.log(new Date().toISOString().slice(11, 19), ...a);

/** The text of an OpenAI message: a string, or the text parts of a list. */
function textOf(content) {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((p) => (p?.type === "text" ? p.text : p?.type === "image_url" ? "[image not passed on]" : ""))
      .filter(Boolean)
      .join("\n");
  }
  return "";
}

/** System prompt, and the conversation as one prompt Claude answers the end of. */
function toPrompt(messages) {
  const system = messages.filter((m) => m.role === "system" || m.role === "developer").map((m) => textOf(m.content)).join("\n\n");
  const turns = messages.filter((m) => m.role === "user" || m.role === "assistant");
  if (turns.length === 1) return { system, prompt: textOf(turns[0].content) };
  const transcript = turns
    .map((m) => `${m.role === "user" ? "User" : "Assistant"}: ${textOf(m.content)}`)
    .join("\n\n");
  return {
    system,
    prompt: `The conversation so far:\n\n${transcript}\n\nReply as the assistant to the user's last message. Write only the reply.`,
  };
}

/** Runs Claude Code once; calls onText with each piece of the answer. */
function runClaude({ model, system, prompt }, onText) {
  const args = [
    "-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages",
    "--model", model, "--tools", "", "--setting-sources", "", "--strict-mcp-config",
    "--no-session-persistence",
  ];
  if (system) args.push("--system-prompt", system);
  const child = spawn(CLAUDE, args, { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
  const done = new Promise((resolve, reject) => {
    let buf = "", streamed = false, error = "", stderr = "";
    child.stdout.on("data", (chunk) => {
      buf += chunk;
      let nl;
      while ((nl = buf.indexOf("\n")) >= 0) {
        const line = buf.slice(0, nl).trim();
        buf = buf.slice(nl + 1);
        if (!line) continue;
        let ev;
        try { ev = JSON.parse(line); } catch { continue; }
        const d = ev.type === "stream_event" ? ev.event?.delta : null;
        if (d?.type === "text_delta" && d.text) { streamed = true; onText(d.text); }
        if (ev.type === "result") {
          if (ev.is_error) error = String(ev.result || ev.subtype || "Claude Code failed");
          else if (!streamed && ev.result) onText(String(ev.result));
        }
      }
    });
    child.stderr.on("data", (c) => { stderr += c; });
    child.on("error", (e) => reject(new Error(`could not start ${CLAUDE}: ${e.message}`)));
    child.on("close", (code) => {
      if (error) reject(new Error(error));
      else if (code !== 0) reject(new Error(stderr.trim().split("\n").pop() || `claude exited with ${code}`));
      else resolve();
    });
  });
  child.stdin.end(prompt);
  return { done, kill: () => child.kill() };
}

function sendJson(res, status, body) {
  res.writeHead(status, { "Content-Type": "application/json" });
  res.end(JSON.stringify(body));
}

const server = createServer(async (req, res) => {
  const path = (req.url || "").split("?")[0].replace(/\/+$/, "");
  if (req.method === "GET" && (path === "/v1/models" || path === "/models")) {
    return sendJson(res, 200, { object: "list", data: MODELS.map((id) => ({ id, object: "model", owned_by: "anthropic" })) });
  }
  if (req.method !== "POST" || (path !== "/v1/chat/completions" && path !== "/chat/completions")) {
    return sendJson(res, 404, { error: { message: "not found" } });
  }

  let raw = "";
  for await (const chunk of req) {
    raw += chunk;
    if (raw.length > MAX_BODY) return sendJson(res, 413, { error: { message: "request too large" } });
  }
  let body;
  try { body = JSON.parse(raw); } catch { return sendJson(res, 400, { error: { message: "not JSON" } }); }
  const model = MODELS.includes(body.model) ? body.model : "sonnet";
  const { system, prompt } = toPrompt(Array.isArray(body.messages) ? body.messages : []);
  if (!prompt.trim()) return sendJson(res, 400, { error: { message: "no message" } });

  const id = `chatcmpl-${Date.now().toString(36)}`;
  const created = Math.floor(Date.now() / 1000);
  const started = Date.now();
  log(`${model}: ${prompt.length} chars${body.stream ? ", streamed" : ""}`);

  if (body.stream) {
    res.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", Connection: "keep-alive" });
    const chunk = (delta, finish = null) =>
      res.write(`data: ${JSON.stringify({ id, object: "chat.completion.chunk", created, model, choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`);
    chunk({ role: "assistant", content: "" });
    const run = runClaude({ model, system, prompt }, (text) => chunk({ content: text }));
    res.on("close", () => { if (!res.writableEnded) run.kill(); });
    try {
      await run.done;
      chunk({}, "stop");
    } catch (e) {
      log(`error: ${e.message}`);
      chunk({ content: `\n\n[Claude Code: ${e.message}]` }, "stop");
    }
    res.end("data: [DONE]\n\n");
  } else {
    let text = "";
    const run = runClaude({ model, system, prompt }, (t) => { text += t; });
    try {
      await run.done;
    } catch (e) {
      log(`error: ${e.message}`);
      return sendJson(res, 502, { error: { message: `Claude Code: ${e.message}` } });
    }
    sendJson(res, 200, {
      id, object: "chat.completion", created, model,
      choices: [{ index: 0, message: { role: "assistant", content: text }, finish_reason: "stop" }],
    });
  }
  log(`${model}: answered in ${((Date.now() - started) / 1000).toFixed(1)} s`);
});

server.listen(PORT, HOST, () => log(`Claude subscription proxy on http://${HOST}:${PORT} (models: ${MODELS.join(", ")})`));
