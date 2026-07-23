/**
 * Defines the shared frontend data contracts for connection state, quotas,
 * token activity, credits, notifications, and initial application state.
 * These types keep the React UI aligned with the Rust backend DTOs.
 * Made by Heavymask — https://heavymask.com
 */
export type ConnectionStatus =
  | "starting"
  | "needsCodex"
  | "needsAuth"
  | "ready"
  | "reconnecting"
  | "error";

export interface AppState {
  status: ConnectionStatus;
  snapshot: UsageSnapshot | null;
  message: string | null;
  codexVersion: string | null;
  codexPath: string | null;
  autostartEnabled: boolean;
  expanded: boolean;
  updating: boolean;
  refreshIntervalSecs: number;
  checkpointPercentages: number[];
  checkpointNotification: CheckpointNotification | null;
}

export interface CheckpointNotification {
  id: string;
  message: string;
}

export interface UsageSnapshot {
  quotaGroups: QuotaGroup[];
  tokenActivity: TokenActivity;
  credits: CreditState | null;
  updatedAt: number;
  stale: boolean;
}

export interface QuotaGroup {
  id: string;
  name: string;
  primary: boolean;
  planType: string | null;
  windows: QuotaWindow[];
}

export interface QuotaWindow {
  key: string;
  label: string;
  usedPercent: number;
  remainingPercent: number;
  windowDurationMins: number | null;
  resetsAt: number | null;
}

export interface TokenActivity {
  todayTokens: string | null;
  lifetimeTokens: string | null;
  peakDailyTokens: string | null;
}

export interface CreditState {
  hasCredits: boolean;
  unlimited: boolean;
  balance: string | null;
  spendControlReached: boolean;
  individualLimit: SpendControl | null;
}

export interface SpendControl {
  limit: string;
  used: string;
  remainingPercent: number;
  resetsAt: number;
}

export const initialState: AppState = {
  status: "starting",
  snapshot: null,
  message: "Connecting to Codex…",
  codexVersion: null,
  codexPath: null,
  autostartEnabled: true,
  expanded: false,
  updating: false,
  refreshIntervalSecs: 60,
  checkpointPercentages: [50, 20, 10],
  checkpointNotification: null,
};
