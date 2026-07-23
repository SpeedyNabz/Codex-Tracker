// Performs a live, minimal Codex app-server JSON-RPC check to verify that the
// installed CLI exposes the allowance and usage data the application needs.
// Made by Heavymask — https://heavymask.com

import { spawn } from "node:child_process";
import readline from "node:readline";

const executable = process.platform === "win32" ? "codex.exe" : "codex";
const child = spawn(executable, ["app-server", "--stdio"], {
  stdio: ["pipe", "pipe", "ignore"],
  windowsHide: true,
});
const lines = readline.createInterface({ input: child.stdout, crlfDelay: Infinity });
const results = new Map();
let finished = false;

function send(message) {
  child.stdin.write(`${JSON.stringify(message)}\n`);
}

function finish(code, message) {
  if (finished) return;
  finished = true;
  clearTimeout(timer);
  lines.close();
  child.kill();
  if (message) (code === 0 ? console.log : console.error)(message);
  process.exitCode = code;
}

lines.on("line", (line) => {
  let message;
  try {
    message = JSON.parse(line);
  } catch {
    return;
  }
  if (message.id === 1) {
    send({ method: "initialized" });
    send({ method: "account/rateLimits/read", id: 2 });
    send({ method: "account/usage/read", id: 3 });
    return;
  }
  if (message.id === 2 || message.id === 3) {
    if (message.error) {
      finish(1, `Live Codex smoke failed: ${message.error.message ?? "request error"}`);
      return;
    }
    results.set(message.id, message.result);
  }
  if (results.size === 2) {
    const limits = results.get(2);
    const usage = results.get(3);
    const primary = limits?.rateLimitsByLimitId?.codex ?? limits?.rateLimits;
    const windows = [primary?.primary, primary?.secondary].filter(Boolean).length;
    const buckets = Array.isArray(usage?.dailyUsageBuckets)
      ? usage.dailyUsageBuckets.length
      : 0;
    if (!primary || !usage?.summary) {
      finish(1, "Live Codex smoke failed: required response fields were absent.");
      return;
    }
    finish(
      0,
      `Live Codex smoke passed: ${windows} allowance window(s), ${buckets} daily bucket(s).`,
    );
  }
});

child.on("error", (error) => finish(1, `Could not start Codex: ${error.message}`));
child.on("exit", (code) => {
  if (!finished) finish(1, `Codex app-server exited before the smoke completed (${code}).`);
});

send({
  method: "initialize",
  id: 1,
  params: {
    clientInfo: {
      name: "codex_usage_overlay_smoke",
      title: "Codex Tracker Smoke",
      version: "0.1.0",
    },
    capabilities: { experimentalApi: false, requestAttestation: false },
  },
});

const timer = setTimeout(() => finish(1, "Live Codex smoke timed out."), 20_000);
