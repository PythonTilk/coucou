// Writes what the updater reads: release/latest.json, beside the installer it
// points at. (Local to this build; see .github/workflows/fork-release.yml.)
//
//   node scripts/fork-latest.mjs <owner/repo> <tag> <notes file>

import { copyFileSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const [repo, tag, notesFile] = process.argv.slice(2);
if (!repo || !tag) throw new Error("usage: fork-latest.mjs <owner/repo> <tag> [notes file]");

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const { version } = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8"));
const msiDir = join(root, "target", "release", "bundle", "msi");
const built = readdirSync(msiDir).find((f) => f.endsWith(".msi") && f.includes(version));
if (!built) throw new Error(`no ${version} installer in ${msiDir}`);

const outDir = join(root, "release");
mkdirSync(outDir, { recursive: true });
const name = `Coucou-Windows-${version}.msi`;
copyFileSync(join(msiDir, built), join(outDir, name));

const latest = {
  version,
  notes: notesFile ? readFileSync(notesFile, "utf8").trim() : "",
  pub_date: new Date().toISOString(),
  platforms: {
    "windows-x86_64": {
      // The signature made by `tauri build`, for the public key in tauri.conf.json.
      signature: readFileSync(join(msiDir, `${built}.sig`), "utf8").trim(),
      url: `https://github.com/${repo}/releases/download/${tag}/${name}`,
    },
  },
};
writeFileSync(join(outDir, "latest.json"), JSON.stringify(latest, null, 2) + "\n");
console.log(`${name} and latest.json are in ${outDir}`);
