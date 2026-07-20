import { useEffect, useState } from "react";
import {
  beginChatgptLogin,
  chooseCodexExecutable,
  getAppState,
  hideOverlay,
  onAppState,
  refreshUsage,
  setAutostartEnabled,
  setOverlayExpanded,
  setOverlayHeight,
  startOverlayDragging,
  useCodexFromPath,
} from "./api";
import type { AppState, QuotaGroup, QuotaWindow } from "./types";
import { initialState } from "./types";
import "./styles.css";

interface OverlayProps {
  state: AppState;
  now: number;
  actionError?: string | null;
  onRefresh: () => void;
  onLogin: () => void;
  onChooseCodex: () => void;
  onUsePath: () => void;
  onAutostart: (enabled: boolean) => void;
  onExpanded: (expanded: boolean) => void;
  onDrag: () => void;
  onHide: () => void;
}

export default function App() {
  const [state, setState] = useState<AppState>(initialState);
  const [now, setNow] = useState(() => Date.now());
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    let unlisten: (() => void) | undefined;
    void getAppState()
      .then((next) => mounted && setState(next))
      .catch((error) => mounted && setActionError(errorMessage(error)));
    void onAppState((next) => {
      if (mounted) {
        setState(next);
        setActionError(null);
      }
    }).then((cleanup) => {
      if (mounted) unlisten = cleanup;
      else cleanup();
    });
    return () => {
      mounted = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    if (state.expanded) return;

    const frame = window.requestAnimationFrame(() => {
      const height = measureCompactOverlayHeight();
      if (height !== null) {
        void setOverlayHeight(height).catch(() => undefined);
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [actionError, state.expanded, state.message, state.snapshot?.updatedAt, state.status]);

  const run = (action: () => Promise<void>) => {
    setActionError(null);
    void action().catch((error) => setActionError(errorMessage(error)));
  };

  return (
    <Overlay
      state={state}
      now={now}
      actionError={actionError}
      onRefresh={() => run(refreshUsage)}
      onLogin={() => run(beginChatgptLogin)}
      onChooseCodex={() => run(chooseCodexExecutable)}
      onUsePath={() => run(useCodexFromPath)}
      onAutostart={(enabled) => run(() => setAutostartEnabled(enabled))}
      onExpanded={(expanded) => run(() => setOverlayExpanded(expanded))}
      onDrag={() => run(startOverlayDragging)}
      onHide={() => run(hideOverlay)}
    />
  );
}

export function Overlay({
  state,
  now,
  actionError,
  onRefresh,
  onLogin,
  onChooseCodex,
  onUsePath,
  onAutostart,
  onExpanded,
  onDrag,
  onHide,
}: OverlayProps) {
  const mainGroups = state.snapshot?.quotaGroups.filter((group) => group.primary) ?? [];
  const additionalGroups =
    state.snapshot?.quotaGroups.filter((group) => !group.primary) ?? [];
  const statusLabel = statusText(state);

  return (
    <main className={`overlay-shell ${state.snapshot?.stale ? "is-stale" : ""}`}>
      <section className="overlay-card" aria-label="Codex usage overlay">
        <header
          className="titlebar"
          onPointerDown={(event) => {
            if (event.button === 0) onDrag();
          }}
        >
          <div className="brand">
            <span className={`status-dot status-${state.status}`} aria-hidden="true" />
            <div>
              <h1>Codex Usage</h1>
              <p>{statusLabel}</p>
            </div>
          </div>
          <div className="window-actions" onPointerDown={(event) => event.stopPropagation()}>
            <button
              className={`icon-button ${state.updating ? "is-spinning" : ""}`}
              type="button"
              aria-label="Refresh usage"
              title="Refresh now"
              disabled={state.updating || state.status === "needsCodex"}
              onClick={onRefresh}
            >
              <RefreshIcon />
            </button>
            <button
              className="icon-button"
              type="button"
              aria-label="Hide overlay"
              title="Hide to tray"
              onClick={onHide}
            >
              <CloseIcon />
            </button>
          </div>
        </header>

        <div className="content" aria-live="polite">
          {state.status === "needsCodex" ? (
            <ActionState
              title="Codex CLI not found"
              message={state.message ?? "Choose your installed codex.exe to continue."}
              actionLabel="Choose codex.exe"
              onAction={onChooseCodex}
            />
          ) : state.status === "needsAuth" && !state.snapshot ? (
            <ActionState
              title="Sign in to Codex"
              message={state.message ?? "Connect your ChatGPT Codex account to view usage."}
              actionLabel="Sign in with Codex"
              onAction={onLogin}
            />
          ) : !state.snapshot ? (
            <LoadingState message={state.message ?? "Reading Codex usage…"} />
          ) : (
            <>
              {mainGroups.length ? (
                mainGroups.map((group) => (
                  <QuotaGroupView key={group.id} group={group} now={now} />
                ))
              ) : (
                <p className="empty-state">No active Codex allowance windows were returned.</p>
              )}

              {state.snapshot.stale && (
                <div className="stale-banner">Last known usage · reconnecting</div>
              )}

              {state.expanded && (
                <ExpandedDetails
                  state={state}
                  additionalGroups={additionalGroups}
                  now={now}
                  onChooseCodex={onChooseCodex}
                  onUsePath={onUsePath}
                  onAutostart={onAutostart}
                />
              )}
            </>
          )}

          {(actionError || (state.message && state.snapshot)) && (
            <p className="message-banner" role={actionError ? "alert" : "status"}>
              {actionError ?? state.message}
            </p>
          )}
        </div>

        <button
          className="expand-button"
          type="button"
          aria-expanded={state.expanded}
          aria-label={state.expanded ? "Show less detail" : "Show token activity and settings"}
          onClick={() => onExpanded(!state.expanded)}
        >
          <ChevronIcon expanded={state.expanded} />
          {state.expanded ? "Less" : "Details"}
        </button>
      </section>
    </main>
  );
}

function QuotaGroupView({ group, now }: { group: QuotaGroup; now: number }) {
  return (
    <section className="quota-group" aria-label={`${group.name} allowances`}>
      {!group.primary && <h2>{group.name}</h2>}
      {group.windows.map((window) => (
        <QuotaMeter key={window.key} window={window} now={now} />
      ))}
    </section>
  );
}

function measureCompactOverlayHeight(): number | null {
  const card = document.querySelector<HTMLElement>(".overlay-card");
  const content = document.querySelector<HTMLElement>(".content");
  if (!card || !content) return null;

  const previousCardHeight = card.style.height;
  const previousContentFlex = content.style.flex;
  const previousContentOverflow = content.style.overflow;
  card.style.height = "auto";
  content.style.flex = "none";
  content.style.overflow = "visible";
  const height = Math.ceil(card.getBoundingClientRect().height + 8);
  card.style.height = previousCardHeight;
  content.style.flex = previousContentFlex;
  content.style.overflow = previousContentOverflow;
  return height;
}

function QuotaMeter({ window, now }: { window: QuotaWindow; now: number }) {
  const remaining = Math.round(Math.max(0, Math.min(100, window.remainingPercent)));
  const tone = remaining <= 20 ? "danger" : remaining <= 50 ? "warning" : "healthy";
  const reset = formatResetCountdown(window.resetsAt, now);
  return (
    <article className="quota-meter">
      <div className="meter-heading">
        <span>{window.label}</span>
        <strong className={`tone-${tone}`}>{remaining}% remaining</strong>
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-label={`${window.label} remaining`}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={remaining}
      >
        <span className={`progress-fill tone-${tone}`} style={{ width: `${remaining}%` }} />
      </div>
      <p title={formatResetTime(window.resetsAt)}>{reset}</p>
    </article>
  );
}

function ExpandedDetails({
  state,
  additionalGroups,
  now,
  onChooseCodex,
  onUsePath,
  onAutostart,
}: {
  state: AppState;
  additionalGroups: QuotaGroup[];
  now: number;
  onChooseCodex: () => void;
  onUsePath: () => void;
  onAutostart: (enabled: boolean) => void;
}) {
  const snapshot = state.snapshot!;
  const activity = snapshot.tokenActivity;
  const credits = snapshot.credits;
  return (
    <div className="expanded-details">
      {additionalGroups.map((group) => (
        <QuotaGroupView key={group.id} group={group} now={now} />
      ))}

      <section className="stats-grid" aria-label="Token activity">
        <Stat label="Today" value={formatTokens(activity.todayTokens)} />
        <Stat label="Lifetime" value={formatTokens(activity.lifetimeTokens)} />
      </section>

      {credits && (
        <section className="credit-panel">
          <span>Additional credits</span>
          <strong>
            {credits.unlimited
              ? "Unlimited"
              : credits.balance
                ? `${credits.balance} credits`
                : "Available"}
          </strong>
          {credits.individualLimit && (
            <small>
              Spend control: {Math.round(credits.individualLimit.remainingPercent)}% remaining
            </small>
          )}
          {credits.spendControlReached && <small className="danger-copy">Limit reached</small>}
        </section>
      )}

      <section className="settings-panel" aria-label="Overlay settings">
        <label className="toggle-row">
          <span>
            <strong>Start with Windows</strong>
            <small>Keep usage visible after sign-in</small>
          </span>
          <input
            type="checkbox"
            checked={state.autostartEnabled}
            onChange={(event) => onAutostart(event.currentTarget.checked)}
          />
        </label>
        <div className="path-row">
          <span title={state.codexPath ?? undefined}>
            <strong>Codex executable</strong>
            <small>{compactPath(state.codexPath)}</small>
          </span>
          <button type="button" onClick={onChooseCodex}>Choose</button>
        </div>
        <button className="path-reset" type="button" onClick={onUsePath}>
          Use Codex from PATH
        </button>
      </section>

      <footer className="updated-at">
        Updated {formatUpdatedAt(snapshot.updatedAt, now)}
        {state.codexVersion && <span title={state.codexVersion}> · Codex connected</span>}
      </footer>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="stat">
      <span>{label}</span>
      <strong>{value}</strong>
      <small>tokens</small>
    </div>
  );
}

function ActionState({
  title,
  message,
  actionLabel,
  onAction,
}: {
  title: string;
  message: string;
  actionLabel: string;
  onAction: () => void;
}) {
  return (
    <div className="action-state">
      <div className="action-icon" aria-hidden="true">!</div>
      <h2>{title}</h2>
      <p>{message}</p>
      <button type="button" onClick={onAction}>{actionLabel}</button>
    </div>
  );
}

function LoadingState({ message }: { message: string }) {
  return (
    <div className="loading-state">
      <span className="loading-ring" aria-hidden="true" />
      <p>{message}</p>
    </div>
  );
}

function statusText(state: AppState): string {
  if (state.snapshot?.stale) return "Reconnecting · showing last update";
  switch (state.status) {
    case "ready":
      return state.updating ? "Refreshing allowance…" : "Allowance remaining";
    case "needsAuth":
      return "Sign-in required";
    case "needsCodex":
      return "Codex CLI required";
    case "reconnecting":
      return "Reconnecting…";
    case "error":
      return "Usage unavailable";
    default:
      return "Connecting…";
  }
}

export function formatResetCountdown(resetsAt: number | null, now: number): string {
  if (!resetsAt) return "Reset time unavailable";
  const seconds = Math.floor(resetsAt - now / 1_000);
  if (seconds <= 0) return "Reset due · refreshing soon";
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  const minutes = Math.floor((seconds % 3_600) / 60);
  if (days > 0) return `Resets in ${days}d ${hours}h`;
  if (hours > 0) return `Resets in ${hours}h ${minutes}m`;
  return `Resets in ${Math.max(1, minutes)}m`;
}

function formatResetTime(resetsAt: number | null): string | undefined {
  return resetsAt
    ? `Resets ${new Date(resetsAt * 1_000).toLocaleString()}`
    : undefined;
}

export function formatTokens(value: string | null): string {
  if (value === null) return "Unavailable";
  try {
    return new Intl.NumberFormat(undefined, {
      notation: "compact",
      maximumFractionDigits: 2,
    }).format(BigInt(value));
  } catch {
    return "Unavailable";
  }
}

function formatUpdatedAt(updatedAt: number, now: number): string {
  const seconds = Math.max(0, Math.floor(now / 1_000 - updatedAt));
  if (seconds < 10) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  return `${Math.floor(seconds / 60)}m ago`;
}

function compactPath(path: string | null): string {
  if (!path) return "Searching PATH";
  const parts = path.replaceAll("/", "\\").split("\\");
  if (parts.length <= 3) return path;
  return `…\\${parts.slice(-3).join("\\")}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function RefreshIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M20 7v5h-5M4 17v-5h5M6.1 8.5A7 7 0 0 1 18.7 7M17.9 15.5A7 7 0 0 1 5.3 17" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="m7 7 10 10M17 7 7 17" />
    </svg>
  );
}

function ChevronIcon({ expanded }: { expanded: boolean }) {
  return (
    <svg className={expanded ? "expanded" : ""} viewBox="0 0 24 24" aria-hidden="true">
      <path d="m7 10 5 5 5-5" />
    </svg>
  );
}
