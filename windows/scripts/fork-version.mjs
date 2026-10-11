// The version of a build published from this fork. (Local to this build.)
//
// A fork build has to be newer than the one before it, for the updater and for
// Windows Installer alike, and both only understand three plain numbers. So the
// build number goes into the third one, on top of upstream's: upstream 0.4.0
// built for the 7th time is 0.4.7, upstream 0.4.1 built for the 12th is
// 0.4.1012. It only ever grows, as long as upstream's own version does.
//
//   node scripts/fork-version.mjs <build number>          prints the version
//   node scripts/fork-version.mjs <build number> --write  and writes it where the build reads it

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** Upstream's version and a build number → this build's version. */
export function forkVersion(upstream, build) {
  const m = /^(\d+)\.(\d+)\.(\d+)$/.exec(upstream);
  if (!m) throw new Error(`"${upstream}" is not a plain x.y.z version`);
  if (!Number.isInteger(build) || build < 1 || build > 999) {
    throw new Error(`the build number must be 1 to 999, not ${build}`);
  }
  const patch = Number(m[3]) * 1000 + build;
  // Windows Installer's third field stops at 65535.
  if (patch > 65535) throw new Error(`${upstream} build ${build} does not fit a Windows Installer version`);
  return `${m[1]}.${m[2]}.${patch}`;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  const confPath = join(root, "src-tauri", "tauri.conf.json");
  const upstream = JSON.parse(readFileSync(confPath, "utf8")).version;
  // The workflow's run number never stops growing; the version has room for 999.
  const build = ((Number(process.argv[2]) - 1) % 999) + 1;
  const version = forkVersion(upstream, build);

  if (process.argv.includes("--write")) {
    const swap = (path, pattern, replacement) => {
      const before = readFileSync(path, "utf8");
      const after = before.replace(pattern, replacement);
      if (after === before) throw new Error(`no version found in ${path}`);
      writeFileSync(path, after);
    };
    swap(confPath, /("version":\s*")[^"]+(")/, `$1${version}$2`);
    swap(join(root, "package.json"), /("version":\s*")[^"]+(")/, `$1${version}$2`);
    swap(join(root, "Cargo.toml"), /(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/, `$1${version}$2`);
  }
  console.log(version);
}
