// Starts the Vite preview server, runs the Playwright visual suite against it,
// and shuts down both child processes while preserving the test exit code.
// Made by Heavymask — https://heavymask.com

import { spawn } from "node:child_process";
import { setTimeout as delay } from "node:timers/promises";

const host = "127.0.0.1";
const port = 4173;
const baseUrl = `http://${host}:${port}/`;
const node = process.execPath;

function start(command, args) {
  return spawn(command, args, {
    cwd: new URL("..", import.meta.url),
    stdio: "inherit",
    windowsHide: true,
  });
}

async function waitForServer() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const response = await fetch(baseUrl);
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await delay(250);
  }
  throw new Error(`The Playwright preview server did not start at ${baseUrl}`);
}

function stop(child) {
  if (!child || child.exitCode !== null) return;
  child.kill();
}

const vite = start(node, ["node_modules/vite/bin/vite.js", "--host", host, "--port", String(port)]);
let runner;
let exitCode = 1;

try {
  await waitForServer();
  runner = start(node, ["node_modules/@playwright/test/cli.js", "test", ...process.argv.slice(2)]);
  exitCode = await new Promise((resolve, reject) => {
    runner.once("error", reject);
    runner.once("exit", (code, signal) => resolve(code ?? (signal ? 1 : 0)));
  });
} finally {
  stop(runner);
  stop(vite);
}

process.exitCode = exitCode;
