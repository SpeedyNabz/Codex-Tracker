## Yes—but display **percentage remaining**, not “tokens left”

Codex now exposes a suitable local integration interface through **`codex app-server`**. It is the same JSON-RPC interface used to power rich clients such as the official VS Code extension. Your application can launch it as a child process, authenticate the user through Codex, and retrieve live usage information.

For ChatGPT-plan Codex usage, you can obtain:

* Usage percentage for each active quota window
* Duration of each window
* Exact reset timestamp
* Credit balance, when applicable
* Workspace spending-limit information
* Lifetime and daily token activity
* Live notifications when rate-limit information changes

However, OpenAI does **not expose the absolute token capacity of the included plan window**. Therefore, your application can accurately show **“72% remaining”**, but generally cannot truthfully show **“184,291 tokens remaining.”** The rate-limit response exposes `usedPercent`, window duration and reset time rather than an absolute token ceiling.

## Recommended architecture

Build a local desktop or tray application using Electron, Tauri, .NET, Python or another desktop framework:

```text
Your desktop UI
      │
      │ JSON-RPC over stdin/stdout
      ▼
codex app-server --stdio
      │
      ▼
User’s authenticated Codex account
```

A normal hosted website cannot launch and communicate with a process on the user’s computer without a separately installed local companion. A desktop app is the cleanest design.

Use `stdio`, which is the default supported transport. The WebSocket transport is explicitly described as experimental and unsupported for production use.

## The relevant methods

### 1. Current limits

Send:

```json
{
  "method": "account/rateLimits/read",
  "id": 2
}
```

A response can contain:

```json
{
  "id": 2,
  "result": {
    "rateLimits": {
      "limitId": "codex",
      "limitName": "Codex",
      "primary": {
        "usedPercent": 28,
        "windowDurationMins": 300,
        "resetsAt": 1784217600
      },
      "secondary": {
        "usedPercent": 43,
        "windowDurationMins": 10080,
        "resetsAt": 1784736000
      },
      "credits": {
        "hasCredits": true,
        "unlimited": false,
        "balance": "12.50"
      }
    }
  }
}
```

Calculate:

```ts
const remainingPercent = Math.max(0, 100 - window.usedPercent);
```

The response may also contain `rateLimitsByLimitId`, providing separate usage buckets keyed by IDs such as `codex`.

### 2. Live updates

Listen for:

```json
{
  "method": "account/rateLimits/updated",
  "params": {
    "rateLimits": {
      "primary": {
        "usedPercent": 31,
        "windowDurationMins": 300,
        "resetsAt": 1784217600
      }
    }
  }
}
```

These are sparse updates. Either merge them into your previous snapshot or, more safely, call `account/rateLimits/read` again whenever this notification arrives.

### 3. Historical token activity

Send:

```json
{
  "method": "account/usage/read",
  "id": 3
}
```

Its response supports:

```json
{
  "summary": {
    "lifetimeTokens": 18450000,
    "peakDailyTokens": 1270000,
    "longestRunningTurnSec": 740,
    "currentStreakDays": 8,
    "longestStreakDays": 22
  },
  "dailyUsageBuckets": [
    {
      "startDate": "2026-07-16",
      "tokens": 415000
    }
  ]
}
```

The schema provides lifetime totals and daily usage buckets, but not an included-plan token ceiling.

## Minimal Node.js client

```ts
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import readline from "node:readline";

type RpcResponse = {
  id?: number;
  method?: string;
  result?: unknown;
  params?: unknown;
  error?: {
    code: number;
    message: string;
  };
};

class CodexUsageClient {
  private readonly process: ChildProcessWithoutNullStreams;
  private nextId = 1;

  private pending = new Map<
    number,
    {
      resolve: (value: unknown) => void;
      reject: (error: Error) => void;
    }
  >();

  constructor(codexExecutable = process.platform === "win32" ? "codex.exe" : "codex") {
    this.process = spawn(codexExecutable, ["app-server", "--stdio"], {
      stdio: ["pipe", "pipe", "pipe"],
      windowsHide: true,
    });

    const lines = readline.createInterface({
      input: this.process.stdout,
      crlfDelay: Infinity,
    });

    lines.on("line", (line) => {
      try {
        this.handleMessage(JSON.parse(line) as RpcResponse);
      } catch (error) {
        console.error("Invalid Codex JSON-RPC message:", error);
      }
    });

    this.process.stderr.on("data", (data: Buffer) => {
      console.error(`Codex: ${data.toString()}`);
    });

    this.process.on("exit", (code) => {
      const error = new Error(`Codex app-server exited with code ${code}`);

      for (const request of this.pending.values()) {
        request.reject(error);
      }

      this.pending.clear();
    });
  }

  async initialize(): Promise<void> {
    await this.request("initialize", {
      clientInfo: {
        name: "codex_usage_monitor",
        title: "Codex Usage Monitor",
        version: "0.1.0",
      },
    });

    this.notify("initialized");
  }

  getRateLimits(): Promise<unknown> {
    return this.request("account/rateLimits/read");
  }

  getTokenActivity(): Promise<unknown> {
    return this.request("account/usage/read");
  }

  private request(method: string, params?: unknown): Promise<unknown> {
    const id = this.nextId++;

    const message =
      params === undefined
        ? { method, id }
        : { method, id, params };

    this.process.stdin.write(`${JSON.stringify(message)}\n`);

    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
    });
  }

  private notify(method: string, params?: unknown): void {
    const message =
      params === undefined
        ? { method }
        : { method, params };

    this.process.stdin.write(`${JSON.stringify(message)}\n`);
  }

  private handleMessage(message: RpcResponse): void {
    if (message.id !== undefined) {
      const pending = this.pending.get(message.id);
      if (!pending) return;

      this.pending.delete(message.id);

      if (message.error) {
        pending.reject(new Error(message.error.message));
      } else {
        pending.resolve(message.result);
      }

      return;
    }

    if (message.method === "account/rateLimits/updated") {
      console.log("Usage changed:", message.params);

      // Recommended: immediately obtain a complete new snapshot.
      void this.getRateLimits().then((limits) => {
        console.log("Updated complete snapshot:", limits);
      });
    }
  }
}

async function main(): Promise<void> {
  const client = new CodexUsageClient();

  await client.initialize();

  const [limits, tokenActivity] = await Promise.all([
    client.getRateLimits(),
    client.getTokenActivity(),
  ]);

  console.log({ limits, tokenActivity });
}

main().catch((error: unknown) => {
  console.error(error);
  process.exitCode = 1;
});
```

The protocol requires an `initialize` request followed by an `initialized` notification before other calls. Codex also lets you generate matching TypeScript definitions or JSON Schema from the installed version, which is preferable to maintaining handwritten types.

## What your interface should display

A useful dashboard could show:

```text
5-hour allowance
██████████████░░░░░░ 72% remaining
Resets in 2h 14m

Weekly allowance
█████████░░░░░░░░░░░ 57% remaining
Resets Monday at 08:00

Today
415,000 tokens

Lifetime
18.45 million tokens

Additional credits
12.50 credits
```

Label the first two as **allowance remaining**, not tokens remaining. Only display an exact credit balance or token total where the returned data actually provides one.

## Important implementation rules

Do not read or copy authentication files such as `~/.codex/auth.json`. Let `codex app-server` manage the ChatGPT browser or device-code login. The app-server exposes proper account login endpoints and persists and refreshes managed ChatGPT credentials itself.

Generate your protocol types during development:

```bash
codex app-server generate-ts --out ./src/codex-protocol
```

Pin or check the installed Codex version because the generated schema corresponds to that exact version.

The honest product claim would be:

> **Monitor your Codex usage live, including quota percentage remaining, reset times, token history and additional credit balance.**

Avoid claiming exact “tokens left” for ChatGPT-plan allowances unless OpenAI later adds an absolute quota field.