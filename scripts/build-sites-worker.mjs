import { cpSync, existsSync, mkdirSync, readdirSync, renameSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const dist = path.join(repositoryRoot, "dist");
const client = path.join(dist, "client");
const server = path.join(dist, "server");
const workerSource = path.join(repositoryRoot, "apps", "sites-worker", "src");

if (!existsSync(path.join(dist, "index.html"))) {
  throw new Error("The frontend build did not produce dist/index.html.");
}

mkdirSync(client, { recursive: true });
for (const entry of readdirSync(dist, { withFileTypes: true })) {
  if (entry.name === "client" || entry.name === "server" || entry.name === ".openai") continue;
  renameSync(path.join(dist, entry.name), path.join(client, entry.name));
}

mkdirSync(server, { recursive: true });
for (const entry of readdirSync(workerSource, { withFileTypes: true })) {
  if (!entry.isFile() || !entry.name.endsWith(".mjs")) continue;
  if (entry.name.endsWith(".test.mjs")) continue;
  cpSync(path.join(workerSource, entry.name), path.join(server, entry.name));
}
cpSync(path.join(workerSource, "index.mjs"), path.join(server, "index.js"));

const builtManifest = path.join(dist, ".openai");
mkdirSync(builtManifest, { recursive: true });
cpSync(
  path.join(repositoryRoot, ".openai", "hosting.json"),
  path.join(builtManifest, "hosting.json"),
);

if (!existsSync(path.join(server, "index.js"))) {
  throw new Error("The Sites Worker entry point is missing from dist/server.");
}

console.log("Built the Wealthfolio Sites frontend and Worker output.");
