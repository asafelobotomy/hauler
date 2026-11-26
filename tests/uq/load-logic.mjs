// Loads a page's logic.js for Node tests. logic.js starts `.pragma library` and makes no Qt
// calls, so dropping the `.pragma` line leaves plain JavaScript. It runs as the body of a function
// (same realm as the tests, so results compare with deepEqual) and the top-level `function` and
// `var` names are the API it returns. `.import "other.js" as Other` lines (a page's logic split
// over several files) load that file the same way, once, and pass it in as `Other`.
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";

const IMPORT = /^\s*\.import\s+"([^"]+\.js)"\s+as\s+([A-Za-z_$][\w$]*)\s*;?\s*$/;

export function loadLogic(path, cache = new Map()) {
  const full = resolve(path);
  if (cache.has(full)) return cache.get(full);
  const imports = [];
  const source = readFileSync(full, "utf8")
    .split("\n")
    .filter((line) => {
      const m = IMPORT.exec(line);
      if (m) imports.push({ file: join(dirname(full), m[1]), name: m[2] });
      return !/^\s*\.(pragma|import)\b/.test(line);
    })
    .join("\n");
  const names = [...source.matchAll(/^(?:function\s+([A-Za-z_$][\w$]*)|var\s+([A-Za-z_$][\w$]*))/gm)].map((m) => m[1] ?? m[2]);
  const api = {};
  cache.set(full, api);
  const deps = imports.map((i) => loadLogic(i.file, cache));
  const run = new Function(...imports.map((i) => i.name), `${source}\nreturn { ${names.join(", ")} };`);
  return Object.assign(api, run(...deps));
}
