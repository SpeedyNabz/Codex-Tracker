import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AppState } from "./types";

export async function getAppState(): Promise<AppState> {
  return invoke<AppState>("get_app_state");
}

export async function onAppState(
  handler: (state: AppState) => void,
): Promise<UnlistenFn> {
  return listen<AppState>("usage-state-changed", (event) => handler(event.payload));
}

export async function refreshUsage(): Promise<void> {
  await invoke("refresh_usage");
}

export async function beginChatgptLogin(): Promise<void> {
  const result = await invoke<{ authUrl: string }>("begin_chatgpt_login");
  await openUrl(result.authUrl);
}

export async function chooseCodexExecutable(): Promise<void> {
  const selected = await open({
    title: "Choose the installed codex.exe",
    multiple: false,
    directory: false,
    filters: [{ name: "Codex executable", extensions: ["exe"] }],
  });
  if (typeof selected === "string") {
    await invoke("set_codex_executable", { path: selected });
  }
}

export async function useCodexFromPath(): Promise<void> {
  await invoke("set_codex_executable", { path: null });
}

export async function setAutostartEnabled(enabled: boolean): Promise<void> {
  await invoke("set_autostart_enabled", { enabled });
}

export async function setOverlayExpanded(expanded: boolean): Promise<void> {
  await invoke("set_overlay_expanded", { expanded });
}

export async function hideOverlay(): Promise<void> {
  await getCurrentWindow().hide();
}
