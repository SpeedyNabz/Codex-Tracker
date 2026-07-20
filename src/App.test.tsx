import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { formatResetCountdown, formatTokens, Overlay } from "./App";
import type { AppState } from "./types";

const now = Date.UTC(2026, 6, 20, 12, 0, 0);

function readyState(overrides: Partial<AppState> = {}): AppState {
  return {
    status: "ready",
    message: null,
    codexVersion: "codex-cli 0.145.0",
    codexPath: "C:\\Tools\\Codex\\codex.exe",
    autostartEnabled: true,
    expanded: false,
    updating: false,
    snapshot: {
      quotaGroups: [
        {
          id: "codex",
          name: "Codex",
          primary: true,
          planType: "plus",
          windows: [
            {
              key: "primary",
              label: "5-hour allowance",
              usedPercent: 28,
              remainingPercent: 72,
              windowDurationMins: 300,
              resetsAt: now / 1_000 + 8_040,
            },
            {
              key: "secondary",
              label: "Weekly allowance",
              usedPercent: 43,
              remainingPercent: 57,
              windowDurationMins: 10_080,
              resetsAt: now / 1_000 + 172_800,
            },
          ],
        },
      ],
      tokenActivity: {
        todayTokens: "415000",
        lifetimeTokens: "9007199254740993",
        peakDailyTokens: "1000000",
      },
      credits: {
        hasCredits: true,
        unlimited: false,
        balance: "12.50",
        spendControlReached: false,
        individualLimit: null,
      },
      updatedAt: now / 1_000,
      stale: false,
    },
    ...overrides,
  };
}

function props(state: AppState) {
  return {
    state,
    now,
    onRefresh: vi.fn(),
    onLogin: vi.fn(),
    onChooseCodex: vi.fn(),
    onUsePath: vi.fn(),
    onAutostart: vi.fn(),
    onExpanded: vi.fn(),
    onDrag: vi.fn(),
    onHide: vi.fn(),
  };
}

describe("Overlay", () => {
  it("renders all available main allowance windows in compact mode", () => {
    const handlers = props(readyState());
    render(<Overlay {...handlers} />);

    expect(screen.getByText("5-hour allowance")).toBeInTheDocument();
    expect(screen.getByText("72% remaining")).toBeInTheDocument();
    expect(screen.getByText("Weekly allowance")).toBeInTheDocument();
    expect(screen.getByText("57% remaining")).toBeInTheDocument();
    expect(screen.queryByText("Today")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /show token activity/i })).toHaveAttribute(
      "aria-expanded",
      "false",
    );

    fireEvent.pointerDown(screen.getByRole("heading", { name: "Codex Usage" }), {
      button: 0,
    });
    expect(handlers.onDrag).toHaveBeenCalledOnce();

    fireEvent.pointerDown(screen.getByRole("button", { name: "Hide overlay" }), {
      button: 0,
    });
    expect(handlers.onDrag).toHaveBeenCalledOnce();
    fireEvent.click(screen.getByRole("button", { name: "Hide overlay" }));
    expect(handlers.onHide).toHaveBeenCalledOnce();
  });

  it("shows exact token activity, credits, and settings only when expanded", () => {
    const state = readyState({ expanded: true });
    const handlers = props(state);
    render(<Overlay {...handlers} />);

    expect(screen.getByText("Today")).toBeInTheDocument();
    expect(screen.getByText(formatTokens("415000"))).toBeInTheDocument();
    expect(screen.getByText(formatTokens("9007199254740993"))).toBeInTheDocument();
    expect(screen.getByText("12.50 credits")).toBeInTheDocument();
    const startup = screen.getByRole("checkbox", { name: /start with windows/i });
    expect(startup).toBeChecked();
    fireEvent.click(startup);
    expect(handlers.onAutostart).toHaveBeenCalledWith(false);
  });

  it("renders missing Codex and sign-in recovery actions", () => {
    const missing = readyState({
      status: "needsCodex",
      snapshot: null,
      message: "Install Codex or choose codex.exe.",
    });
    const missingHandlers = props(missing);
    const { rerender } = render(<Overlay {...missingHandlers} />);
    fireEvent.click(screen.getByRole("button", { name: "Choose codex.exe" }));
    expect(missingHandlers.onChooseCodex).toHaveBeenCalledOnce();

    const auth = readyState({
      status: "needsAuth",
      snapshot: null,
      message: "Sign in with Codex.",
    });
    const authHandlers = props(auth);
    rerender(<Overlay {...authHandlers} />);
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Codex" }));
    expect(authHandlers.onLogin).toHaveBeenCalledOnce();
  });

  it("marks retained data as stale without removing it", () => {
    const state = readyState({
      status: "reconnecting",
      snapshot: { ...readyState().snapshot!, stale: true },
    });
    render(<Overlay {...props(state)} />);
    expect(screen.getByText("Last known usage · reconnecting")).toBeInTheDocument();
    expect(screen.getByText("72% remaining")).toBeInTheDocument();
  });

  it("omits credits when the server does not provide them", () => {
    const base = readyState({ expanded: true });
    const state = {
      ...base,
      snapshot: { ...base.snapshot!, credits: null },
    };
    render(<Overlay {...props(state)} />);
    expect(screen.queryByText("Additional credits")).not.toBeInTheDocument();
  });
});

describe("formatting", () => {
  it("formats countdown rollover and absent reset times", () => {
    expect(formatResetCountdown(null, now)).toBe("Reset time unavailable");
    expect(formatResetCountdown(now / 1_000 - 1, now)).toBe(
      "Reset due · refreshing soon",
    );
    expect(formatResetCountdown(now / 1_000 + 8_040, now)).toBe("Resets in 2h 14m");
    expect(formatResetCountdown(now / 1_000 + 172_800, now)).toBe("Resets in 2d 0h");
  });

  it("formats bigint decimal strings without first converting to Number", () => {
    expect(formatTokens("9007199254740993")).not.toBe("Unavailable");
    expect(formatTokens("not-a-number")).toBe("Unavailable");
    expect(formatTokens(null)).toBe("Unavailable");
  });
});
