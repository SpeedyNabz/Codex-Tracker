/**
 * Renders the Codex Tracker overlay and coordinates its UI state, controls,
 * themes, quota presentation, and user actions through the Tauri API layer.
 * This is the main frontend surface of the project.
 * Made by Heavymask — https://heavymask.com
 */
import { useEffect, useState } from "react";
import type { FormEvent } from "react";
import {
  beginChatgptLogin,
  chooseCodexExecutable,
  dismissCheckpointNotification,
  getAppState,
  hideOverlay,
  openHeavyMaskSite,
  onAppState,
  refreshUsage,
  setAutostartEnabled,
  setCheckpointPercentages,
  setOverlayExpanded,
  setOverlayHeight,
  setRefreshInterval,
  startOverlayDragging,
  useCodexFromPath,
} from "./api";
import type { AppState, QuotaGroup, QuotaWindow } from "./types";
import { initialState } from "./types";
import sammyLogo from "./assets/heavymask-sammy.svg";
import "./styles.css";

export type Theme = "dark" | "light";
export type Palette = "heavymask" | "ocean" | "orchid" | "ember" | "forest";

const THEME_STORAGE_KEY = "codex-usage-overlay-theme";
const PALETTE_STORAGE_KEY = "codex-usage-overlay-palette";

export const PALETTE_OPTIONS: Array<{ id: Palette; label: string; description: string }> = [
  { id: "heavymask", label: "HeavyMask", description: "Yellow" },
  { id: "ocean", label: "Ocean", description: "Blue + aqua" },
  { id: "orchid", label: "Orchid", description: "Violet + pink" },
  { id: "ember", label: "Ember", description: "Amber + coral" },
  { id: "forest", label: "Forest", description: "Mint + green" },
];

interface OverlayProps {
  state: AppState;
  now: number;
  theme: Theme;
  palette: Palette;
  actionError?: string | null;
  onRefresh: () => void;
  onLogin: () => void;
  onChooseCodex: () => void;
  onUsePath: () => void;
  onAutostart: (enabled: boolean) => void;
  onRefreshInterval: (seconds: number) => void;
  onCheckpointPercentages: (percentages: number[]) => void;
  onDismissCheckpointNotification: () => void;
  onExpanded: (expanded: boolean) => void;
  onDrag: () => void;
  onHide: () => void;
  onToggleTheme: () => void;
  onPaletteChange: (palette: Palette) => void;
  onOpenHeavyMask: () => void;
}

export default function App() {
  const [state, setState] = useState<AppState>(initialState);
  const [now, setNow] = useState(() => Date.now());
  const [theme, setTheme] = useState<Theme>(loadTheme);
  const [palette, setPalette] = useState<Palette>(loadPalette);
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
    try {
      window.localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      // Local storage can be unavailable in restricted webviews.
    }
  }, [theme]);

  useEffect(() => {
    try {
      window.localStorage.setItem(PALETTE_STORAGE_KEY, palette);
    } catch {
      // Local storage can be unavailable in restricted webviews.
    }
  }, [palette]);

  useEffect(() => {
    if (state.expanded) return;

    const frame = window.requestAnimationFrame(() => {
      const height = measureCompactOverlayHeight();
      if (height !== null) {
        void setOverlayHeight(height).catch(() => undefined);
      }
    });
    return () => window.cancelAnimationFrame(frame);
  }, [
    actionError,
    state.checkpointNotification?.id,
    state.expanded,
    state.message,
    state.snapshot?.updatedAt,
    state.status,
  ]);

  const run = (action: () => Promise<void>) => {
    setActionError(null);
    void action().catch((error) => setActionError(errorMessage(error)));
  };

  return (
    <Overlay
      state={state}
      now={now}
      theme={theme}
      palette={palette}
      actionError={actionError}
      onRefresh={() => run(refreshUsage)}
      onLogin={() => run(beginChatgptLogin)}
      onChooseCodex={() => run(chooseCodexExecutable)}
      onUsePath={() => run(useCodexFromPath)}
      onAutostart={(enabled) => run(() => setAutostartEnabled(enabled))}
      onRefreshInterval={(seconds) => run(() => setRefreshInterval(seconds))}
      onCheckpointPercentages={(percentages) => run(() => setCheckpointPercentages(percentages))}
      onDismissCheckpointNotification={() => run(dismissCheckpointNotification)}
      onExpanded={(expanded) => run(() => setOverlayExpanded(expanded))}
      onDrag={() => run(startOverlayDragging)}
      onHide={() => run(hideOverlay)}
      onToggleTheme={() => setTheme((current) => (current === "dark" ? "light" : "dark"))}
      onPaletteChange={setPalette}
      onOpenHeavyMask={() => run(openHeavyMaskSite)}
    />
  );
}

export function Overlay({
  state,
  now,
  theme,
  palette,
  actionError,
  onRefresh,
  onLogin,
  onChooseCodex,
  onUsePath,
  onAutostart,
  onRefreshInterval,
  onCheckpointPercentages,
  onDismissCheckpointNotification,
  onExpanded,
  onDrag,
  onHide,
  onToggleTheme,
  onPaletteChange,
  onOpenHeavyMask,
}: OverlayProps) {
  const mainGroups = state.snapshot?.quotaGroups.filter((group) => group.primary) ?? [];
  const additionalGroups =
    state.snapshot?.quotaGroups.filter((group) => !group.primary) ?? [];
  const statusLabel = statusText(state);

  return (
    <main
      className={`overlay-shell theme-${theme} palette-${palette} ${state.snapshot?.stale ? "is-stale" : ""}`}
    >
      <section className="overlay-card" aria-label="Codex Tracker overlay">
        <header
          className="titlebar"
          onPointerDown={(event) => {
            if (event.button === 0) onDrag();
          }}
        >
          <div className="brand">
            <span className="hm-mark" aria-hidden="true">
              <img src={sammyLogo} alt="" />
            </span>
            <span className={`status-dot status-${state.status}`} aria-hidden="true" />
            <div>
              <h1>Codex Tracker</h1>
              <p>{statusLabel}</p>
            </div>
          </div>
          <div className="window-actions" onPointerDown={(event) => event.stopPropagation()}>
            <div className="refresh-control">
              <span
                className="refresh-age"
                aria-label={
                  state.snapshot
                    ? `Last refreshed ${formatUpdatedAt(state.snapshot.updatedAt, now)}`
                    : "No refresh completed yet"
                }
                title={
                  state.snapshot
                    ? `Last refreshed ${formatUpdatedAt(state.snapshot.updatedAt, now)}`
                    : "No refresh completed yet"
                }
              >
                {state.snapshot ? formatUpdatedAt(state.snapshot.updatedAt, now) : "Not yet"}
              </span>
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
            </div>
            <button
              className="icon-button"
              type="button"
              aria-label={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              title={theme === "dark" ? "Switch to light mode" : "Switch to dark mode"}
              onClick={onToggleTheme}
            >
              {theme === "dark" ? <SunIcon /> : <MoonIcon />}
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
          {state.checkpointNotification && (
            <div className="checkpoint-banner" role="alert">
              <span>{state.checkpointNotification.message}</span>
              <button
                type="button"
                aria-label="Dismiss checkpoint notification"
                onClick={onDismissCheckpointNotification}
              >
                <CloseIcon />
              </button>
            </div>
          )}
          {state.status === "needsCodex" ? (
            <ActionState
              title="Codex CLI not found"
              message={state.message ?? "Choose your installed Codex executable to continue."}
              actionLabel="Choose Codex executable"
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
                  onRefreshInterval={onRefreshInterval}
                  onCheckpointPercentages={onCheckpointPercentages}
                  palette={palette}
                  onPaletteChange={onPaletteChange}
                  onOpenHeavyMask={onOpenHeavyMask}
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

function loadTheme(): Theme {
  try {
    return window.localStorage.getItem(THEME_STORAGE_KEY) === "light" ? "light" : "dark";
  } catch {
    return "dark";
  }
}

function loadPalette(): Palette {
  try {
    const stored = window.localStorage.getItem(PALETTE_STORAGE_KEY);
    return PALETTE_OPTIONS.some((option) => option.id === stored)
      ? (stored as Palette)
      : "heavymask";
  } catch {
    return "heavymask";
  }
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
  onRefreshInterval,
  onCheckpointPercentages,
  palette,
  onPaletteChange,
  onOpenHeavyMask,
}: {
  state: AppState;
  additionalGroups: QuotaGroup[];
  now: number;
  onChooseCodex: () => void;
  onUsePath: () => void;
  onAutostart: (enabled: boolean) => void;
  onRefreshInterval: (seconds: number) => void;
  onCheckpointPercentages: (percentages: number[]) => void;
  palette: Palette;
  onPaletteChange: (palette: Palette) => void;
  onOpenHeavyMask: () => void;
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

      <div className="maker-credit">
        <span className="maker-signature">
          <span className="maker-logo" aria-hidden="true" />
          <span>Made by <strong>HeavyMask</strong></span>
        </span>
        <button
          className="maker-link"
          type="button"
          aria-label="Open HeavyMask website"
          onClick={onOpenHeavyMask}
        >
          heavymask.com
          <ExternalLinkIcon />
        </button>
      </div>

      <section className="settings-panel" role="region" aria-label="Extended settings">
        <div className="appearance-setting">
          <div className="appearance-setting-header">
            <span>
              <strong>Themes</strong>
              <small>Choose an accent theme</small>
            </span>
            <output>{PALETTE_OPTIONS.find((option) => option.id === palette)?.label}</output>
          </div>
          <div className="palette-grid" role="group" aria-label="Theme options">
            {PALETTE_OPTIONS.map((option) => (
              <button
                key={option.id}
                className={`palette-option ${palette === option.id ? "is-selected" : ""}`}
                type="button"
                aria-pressed={palette === option.id}
                aria-label={`Select ${option.label} theme`}
                onClick={() => onPaletteChange(option.id)}
              >
                <span className={`palette-swatch palette-swatch-${option.id}`} aria-hidden="true" />
                <span className="palette-option-copy">
                  <strong>{option.label}</strong>
                  <small>{option.description}</small>
                </span>
              </button>
            ))}
          </div>
        </div>
        <CheckpointSettings
          percentages={state.checkpointPercentages}
          onSave={onCheckpointPercentages}
        />
        <div className="refresh-setting">
          <div className="refresh-setting-header">
            <span>
              <strong>Auto-refresh</strong>
              <small>How often usage is checked</small>
            </span>
            <output>{formatRefreshInterval(snapshotRefreshInterval(state))}</output>
          </div>
          <input
            aria-label="Auto-refresh interval"
            type="range"
            min={15}
            max={300}
            step={15}
            value={snapshotRefreshInterval(state)}
            onChange={(event) => onRefreshInterval(Number(event.currentTarget.value))}
          />
          <div className="range-labels" aria-hidden="true">
            <span>15s</span>
            <span>5m</span>
          </div>
        </div>
        <label className="toggle-row">
          <span>
            <strong>Start at login</strong>
            <small>Keep usage visible after you sign in</small>
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

function CheckpointSettings({
  percentages,
  onSave,
}: {
  percentages: number[];
  onSave: (percentages: number[]) => void;
}) {
  const serializedPercentages = percentages.join(",");
  const [draft, setDraft] = useState(() => percentages.join(", "));
  const [submitted, setSubmitted] = useState(false);
  const parsed = parseCheckpointPercentages(draft);
  const hasInvalidValue = draft
    .trim()
    .split(/[\s,]+/)
    .filter(Boolean)
    .some((value) => !/^\d+$/.test(value) || Number(value) < 1 || Number(value) > 99);

  useEffect(() => {
    setDraft(percentages.join(", "));
    setSubmitted(false);
  }, [serializedPercentages]);

  function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setSubmitted(true);
    if (!hasInvalidValue) onSave(parsed);
  }

  return (
    <form className="checkpoint-setting" onSubmit={save}>
      <div className="checkpoint-setting-header">
        <span>
          <strong>Checkpoints</strong>
          <small>Notify when remaining usage reaches a level</small>
        </span>
        <output>{percentages.length ? `${percentages.length} active` : "Off"}</output>
      </div>
      <div className="checkpoint-input-row">
        <input
          aria-label="Checkpoint percentages"
          type="text"
          inputMode="numeric"
          value={draft}
          placeholder="50, 20, 10"
          onChange={(event) => setDraft(event.currentTarget.value)}
        />
        <button type="submit">Save</button>
      </div>
      <small className="checkpoint-help">Remaining % · comma-separated · 1–99. Leave blank to turn off.</small>
      {submitted && hasInvalidValue && (
        <small className="checkpoint-error" role="alert">Use whole percentages from 1 to 99.</small>
      )}
    </form>
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

function snapshotRefreshInterval(state: AppState): number {
  return Math.max(15, Math.min(300, state.refreshIntervalSecs));
}

export function formatRefreshInterval(seconds: number): string {
  if (seconds % 60 === 0) return `Every ${seconds / 60}m`;
  return `Every ${seconds}s`;
}

export function parseCheckpointPercentages(value: string): number[] {
  return Array.from(
    new Set(
      value
        .split(/[\s,]+/)
        .filter(Boolean)
        .map(Number)
        .filter((percentage) => Number.isInteger(percentage) && percentage >= 1 && percentage <= 99),
    ),
  ).sort((left, right) => right - left);
}

function compactPath(path: string | null): string {
  if (!path) return "Searching PATH";
  const separator = path.includes("\\") ? "\\" : "/";
  const parts = path.split(/[\\/]/);
  if (parts.length <= 3) return path;
  return `…${separator}${parts.slice(-3).join(separator)}`;
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

function ExternalLinkIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M13 5h6v6M19 5l-8 8M18 14v4a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h4" />
    </svg>
  );
}

function SunIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <circle cx="12" cy="12" r="3.5" />
      <path d="M12 2.5v2M12 19.5v2M4.7 4.7l1.4 1.4M17.9 17.9l1.4 1.4M2.5 12h2M19.5 12h2M4.7 19.3l1.4-1.4M17.9 6.1l1.4-1.4" />
    </svg>
  );
}

function MoonIcon() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M19.5 15.2A7.5 7.5 0 0 1 8.8 4.5 7.5 7.5 0 1 0 19.5 15.2Z" />
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
