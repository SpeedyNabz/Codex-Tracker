/**
 * Bootstraps the React application by mounting the Codex Tracker overlay into
 * the document root with React Strict Mode enabled.
 * Made by Heavymask — https://heavymask.com
 */
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
