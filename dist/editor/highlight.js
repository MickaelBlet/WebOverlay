// Syntax highlighting: sticky regex rules tried in order at each position.

import { escapeHtml } from "./core.js";

const STRING = [/"(?:[^"\\\n]|\\.)*"?/y, "str"];
const SQ_STRING = [/'(?:[^'\\\n]|\\.)*'?/y, "str"];
const BLOCK_COMMENT = [/\/\*[\s\S]*?(?:\*\/|$)/y, "com"];
const LANGS = {
  toml: [
    [/#.*/y, "com"],
    [/"""[\s\S]*?(?:"""|$)/y, "str"],
    [/'''[\s\S]*?(?:'''|$)/y, "str"],
    STRING,
    [/'[^'\n]*'?/y, "str"],
    [/^[ \t]*\[\[?[^\[\]\n]*\]\]?(?=[ \t]*(?:#|$))/my, "sec"],
    [/[A-Za-z0-9_.-]+(?=[ \t]*=)/y, "key"],
    [/\d{4}-\d{2}-\d{2}(?:[T ]\d{2}:\d{2}(?::\d{2}(?:\.\d+)?)?(?:Z|[+-]\d{2}:\d{2})?)?/y, "num"],
    [/[+-]?(?:0x[\da-fA-F_]+|0o[0-7_]+|0b[01_]+|inf\b|nan\b|\d[\d_]*(?:\.\d[\d_]*)?(?:[eE][+-]?\d+)?)/y, "num"],
    [/(?:true|false)\b/y, "bool"],
    [/[A-Za-z_][\w-]*/y, null],
  ],
  js: [
    [/\/\/.*/y, "com"],
    BLOCK_COMMENT,
    [/`(?:[^`\\]|\\[\s\S])*`?/y, "str"],
    STRING,
    SQ_STRING,
    [/(?:async|await|break|case|catch|class|const|continue|debugger|default|delete|do|else|export|extends|finally|for|function|if|import|in|instanceof|let|new|of|return|static|super|switch|this|throw|try|typeof|var|void|while|with|yield)\b/y, "kw"],
    [/(?:true|false|null|undefined|NaN|Infinity)\b/y, "bool"],
    [/(?:0[xX][\da-fA-F_]+|0[bB][01_]+|0[oO][0-7_]+|\d[\d_]*(?:\.\d*)?(?:[eE][+-]?\d+)?n?|\.\d+(?:[eE][+-]?\d+)?)/y, "num"],
    [/[A-Za-z_$][\w$]*(?=\s*\()/y, "fn"],
    [/[A-Za-z_$][\w$]*/y, null],
  ],
  css: [
    BLOCK_COMMENT,
    STRING,
    SQ_STRING,
    [/@[\w-]+/y, "kw"],
    [/!\s*important\b/y, "kw"],
    [/(?<=[{;]\s*)-*[A-Za-z][\w-]*(?=\s*:)/y, "key"],
    [/#[\da-fA-F]{3,8}\b/y, "num"],
    [/-?(?:\d+\.?\d*|\.\d+)(?:%|[A-Za-z]+)?/y, "num"],
    [/[.#][A-Za-z_-][\w-]*/y, "fn"],
    [/[A-Za-z_-][\w-]*/y, null],
  ],
};

export function highlight(src, lang) {
  const rules = LANGS[lang];
  if (!rules) return escapeHtml(src);
  let out = "";
  let plain = "";
  let pos = 0;
  outer: while (pos < src.length) {
    for (const [re, cls] of rules) {
      re.lastIndex = pos;
      const m = re.exec(src);
      if (!m || m[0].length === 0) continue;
      if (cls) {
        out += escapeHtml(plain) + `<span class="${cls}">${escapeHtml(m[0])}</span>`;
        plain = "";
      } else {
        plain += m[0];
      }
      pos += m[0].length;
      continue outer;
    }
    plain += src[pos++];
  }
  return out + escapeHtml(plain);
}
