/**
 * Runs the same validated test/build/copy release flow used by the Windows
 * PowerShell builder, using native macOS or Linux Tauri bundlers.
 * Made by HeavyMask — https://heavymask.com
 */
import {
  copyFileSync,
  mkdirSync,
  readdirSync,
  statSync,
} from "node:fs";
import { basename, join, resolve } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
const artifactDirectory = join(repositoryRoot, "artifacts");
const platform = process.argv[2];
const skipTests = process.argv.includes("--skip-tests");
const npmCommand = process.platform === "win32" ? "npm.cmd" : "npm";

const releaseDefinitions = {
  macos: {
    host: "darwin",
    buildArguments: [
      "run",
      "tauri",
      "--",
      "build",
      "--target",
      "universal-apple-darwin",
      "--bundles",
      "dmg",
    ],
    bundleDirectories: [
      {
        directory: join(
          repositoryRoot,
          "src-tauri",
          "target",
          "universal-apple-darwin",
          "release",
          "bundle",
          "dmg",
        ),
        extensions: [".dmg"],
      },
    ],
  },
  linux: {
    host: "linux",
    buildArguments: [
      "run",
      "tauri",
      "--",
      "build",
      "--bundles",
      "appimage,deb",
    ],
    bundleDirectories: [
      {
        directory: join(
          repositoryRoot,
          "src-tauri",
          "target",
          "release",
          "bundle",
          "appimage",
        ),
        extensions: [".AppImage"],
      },
      {
        directory: join(
          repositoryRoot,
          "src-tauri",
          "target",
          "release",
          "bundle",
          "deb",
        ),
        extensions: [".deb"],
      },
    ],
  },
};

const definition = releaseDefinitions[platform];
if (!definition) {
  throw new Error("Usage: node scripts/build-desktop-release.mjs <macos|linux> [--skip-tests]");
}
if (process.platform !== definition.host) {
  throw new Error(
    `${platform} releases must be built on a native ${definition.host} host; current host is ${process.platform}.`,
  );
}

function run(command, arguments_) {
  const result = spawnSync(command, arguments_, {
    cwd: repositoryRoot,
    stdio: "inherit",
    windowsHide: true,
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${command} ${arguments_.join(" ")} failed with exit code ${result.status}.`);
  }
}

if (!skipTests) run(npmCommand, ["test"]);
run(npmCommand, definition.buildArguments);

mkdirSync(artifactDirectory, { recursive: true });
const artifacts = [];
for (const bundle of definition.bundleDirectories) {
  const matchingFiles = readdirSync(bundle.directory)
    .filter((file) => bundle.extensions.some((extension) => file.endsWith(extension)))
    .map((file) => join(bundle.directory, file))
    .filter((file) => statSync(file).isFile());

  if (matchingFiles.length === 0) {
    throw new Error(`No release bundle was created in ${bundle.directory}.`);
  }

  for (const source of matchingFiles) {
    const releaseName = basename(source).replaceAll(" ", ".");
    const destination = join(artifactDirectory, releaseName);
    copyFileSync(source, destination);
    artifacts.push(destination);
  }
}

console.log("Build complete.");
for (const artifact of artifacts) console.log(`Release artifact: ${artifact}`);
