// A reader for the TOML subset page.toml uses: key = value lines, [table] and [[array of tables]]
// headers, strings (basic and literal), integers, booleans, arrays and inline tables. It exists
// so the tests need no dependency; it throws on anything else rather than guess.
export function parseToml(text) {
  const root = {};
  let table = root;
  const lines = text.split("\n");
  for (let n = 0; n < lines.length; n++) {
    const line = stripComment(lines[n]).trim();
    if (line === "") continue;
    let m = /^\[\[([A-Za-z0-9_.-]+)\]\]$/.exec(line);
    if (m) {
      (root[m[1]] ??= []).push((table = {}));
      continue;
    }
    m = /^\[([A-Za-z0-9_.-]+)\]$/.exec(line);
    if (m) {
      table = root[m[1]] = {};
      continue;
    }
    m = /^([A-Za-z0-9_-]+)\s*=\s*(.*)$/.exec(line);
    if (!m) throw new Error(`page.toml:${n + 1}: cannot read: ${line}`);
    const [value, rest] = readValue(m[2], n + 1);
    if (rest.trim() !== "") throw new Error(`page.toml:${n + 1}: trailing text: ${rest}`);
    table[m[1]] = value;
  }
  return root;
}

// Drops a `#` comment, ignoring `#` inside strings.
function stripComment(line) {
  let quote = null;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (quote) {
      if (quote === '"' && c === "\\") i++;
      else if (c === quote) quote = null;
    } else if (c === '"' || c === "'") quote = c;
    else if (c === "#") return line.slice(0, i);
  }
  return line;
}

function readValue(s, n) {
  s = s.trimStart();
  const c = s[0];
  if (c === '"') {
    let i = 1;
    let out = "";
    for (; i < s.length && s[i] !== '"'; i++) {
      if (s[i] === "\\") {
        const e = s[++i];
        const map = { n: "\n", t: "\t", '"': '"', "\\": "\\" };
        if (!(e in map)) throw new Error(`page.toml:${n}: unsupported escape \\${e}`);
        out += map[e];
      } else out += s[i];
    }
    return [out, s.slice(i + 1)];
  }
  if (c === "'") {
    const end = s.indexOf("'", 1);
    if (end < 0) throw new Error(`page.toml:${n}: unterminated string`);
    return [s.slice(1, end), s.slice(end + 1)];
  }
  if (c === "[" || c === "{") {
    const close = c === "[" ? "]" : "}";
    const items = c === "[" ? [] : {};
    s = s.slice(1);
    for (;;) {
      s = s.trimStart();
      if (s[0] === close) return [items, s.slice(1)];
      if (c === "[") {
        const [v, rest] = readValue(s, n);
        items.push(v);
        s = rest;
      } else {
        const m = /^([A-Za-z0-9_-]+)\s*=\s*/.exec(s);
        if (!m) throw new Error(`page.toml:${n}: bad inline table near ${s}`);
        const [v, rest] = readValue(s.slice(m[0].length), n);
        items[m[1]] = v;
        s = rest;
      }
      s = s.trimStart();
      if (s[0] === ",") s = s.slice(1);
      else if (s[0] !== close) throw new Error(`page.toml:${n}: expected , or ${close}`);
    }
  }
  let m = /^(true|false)\b/.exec(s);
  if (m) return [m[1] === "true", s.slice(m[0].length)];
  m = /^-?[0-9_]+/.exec(s);
  if (m) return [Number(m[0].replace(/_/g, "")), s.slice(m[0].length)];
  throw new Error(`page.toml:${n}: cannot read value: ${s}`);
}
