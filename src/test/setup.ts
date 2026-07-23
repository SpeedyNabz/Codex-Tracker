/**
 * Configures the shared Vitest browser-like test environment and cleans up
 * rendered Testing Library components after each test.
 * Made by Heavymask — https://heavymask.com
 */
import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";

afterEach(cleanup);
