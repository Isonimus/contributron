#!/usr/bin/env node
// Enforces standing invariant 10 (ADR-0013): no secret in any tracked file, and no Rust
// code path that writes a SecretString-derived buffer into a file inside the working tree.
//
// Zero dependencies by design (same reason as scripts/lint-docs.mjs). Scans the tracked
// corpus reported by `git ls-files` so an untracked local scratch file does not green or
// red a commit; the pre-commit hook runs this against the staged tree.
//
//   node scripts/lint-secrets.mjs [repo-root]      (default: cwd)
//
// Exit 1 on any finding.

import { readFileSync, readdirSync, existsSync, statSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { join, resolve, relative, sep } from 'node:path';

const root = resolve(process.argv[2] ?? process.cwd());

// Token/key shapes per ADR-0013 and slice 0014. Patterns match live credentials; the
// literal text of a *pattern* (with its character classes and quantifier syntax) is not a
// credential and so does not match itself. The PEM opener, however, is literal text that
// does match itself: files that legitimately quote it as documentation go on the file
// allowlist below, not into the pattern.
const PATTERNS = [
  { name: 'github personal access token (classic)', re: /ghp_[A-Za-z0-9]{36,}/ },
  { name: 'github fine-grained token', re: /github_pat_[A-Za-z0-9_]{80,}/ },
  { name: 'github oauth access token', re: /gho_[A-Za-z0-9]{36,}/ },
  { name: 'github user access token', re: /ghu_[A-Za-z0-9]{36,}/ },
  { name: 'github server-to-server token', re: /ghs_[A-Za-z0-9]{36,}/ },
  { name: 'github refresh token', re: /ghr_[A-Za-z0-9]{36,}/ },
  { name: 'PEM private key header', re: /-----BEGIN (RSA |OPENSSH |EC )?PRIVATE KEY-----/ },
];

// Files that quote a pattern as documentation. A path lands here only when its content
// has been read and confirmed to be prose about the shape, not a secret of that shape.
const DOC_ALLOWLIST = new Set([
  'scripts/lint-secrets.mjs',
  'slices/0014-react-to-all-review-threads-resolved.md',
  'adr/0013-operator-personal-access-token-for-m2-github-app-deferred.md',
]);

// Rust write-call shapes that would land a buffer on disk inside the working tree. The
// check is syntactic: a hit means the write call's own argument list references a secret
// handle. The argument list is extracted by paren-matching from the opening `(` so an
// unrelated `.expose()` elsewhere in the function body is not a false positive. A pass
// means the patterns are absent from write-call arguments; the SecretString wrapper is
// the real defence (slice 0014, design).
const WRITE_CALLS = /\b(?:std::fs::write|fs::write|File::create|tokio::fs::write)\s*\(/g;
const SECRET_HANDLES = [/\bSecretString\b/, /\.expose\s*\(\s*\)/];

// Extract the argument text between a call's `(` at `open` and its matching `)`, honouring
// nested parens and skipping over string, char and line-comment content so a `)` inside a
// `"literal"` does not close the call. Returns null if the call is unterminated.
function extractCallArgs(text, open) {
  let depth = 1;
  let i = open + 1;
  const start = i;
  while (i < text.length && depth > 0) {
    const ch = text[i];
    if (ch === '"') {
      i += 1;
      while (i < text.length) {
        if (text[i] === '\\') { i += 2; continue; }
        if (text[i] === '"') { i += 1; break; }
        i += 1;
      }
      continue;
    }
    if (ch === "'") {
      i += 1;
      while (i < text.length) {
        if (text[i] === '\\') { i += 2; continue; }
        if (text[i] === "'") { i += 1; break; }
        i += 1;
      }
      continue;
    }
    if (ch === '/' && text[i + 1] === '/') {
      while (i < text.length && text[i] !== '\n') i += 1;
      continue;
    }
    if (ch === '(') depth += 1;
    else if (ch === ')') {
      depth -= 1;
      if (depth === 0) return text.slice(start, i);
    }
    i += 1;
  }
  return null;
}

// Scanning strategy depends on where the linter is invoked. A working tree has `.git` and
// we trust `git ls-files` so untracked scratch files don't influence the verdict. The
// pre-commit hook invokes us against the staged tree it extracted with `git archive` into
// a tempdir (ADR-0018), which contains only committed paths and no `.git`; there we walk
// the filesystem, which gives the same answer because everything present was committed.
function tracked() {
  if (existsSync(join(root, '.git'))) {
    const out = execFileSync('git', ['-C', root, 'ls-files', '-z'], { encoding: 'buffer' });
    return out.toString('utf8').split('\0').filter(Boolean);
  }
  const out = [];
  walk(root, out);
  return out;
}

function walk(dir, out) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name === '.git') continue;
    const abs = join(dir, entry.name);
    if (entry.isDirectory()) walk(abs, out);
    else if (entry.isFile()) out.push(relative(root, abs).split(sep).join('/'));
  }
}

function isProbablyBinary(buf) {
  const scan = buf.length > 8000 ? buf.subarray(0, 8000) : buf;
  for (const byte of scan) if (byte === 0) return true;
  return false;
}

const findings = [];

for (const relPath of tracked()) {
  const abs = join(root, relPath);
  let stat;
  try { stat = statSync(abs); } catch { continue; }
  if (!stat.isFile()) continue;
  if (stat.size > 2_000_000) continue;

  const buf = readFileSync(abs);
  if (isProbablyBinary(buf)) continue;
  const text = buf.toString('utf8');

  if (!DOC_ALLOWLIST.has(relPath)) {
    for (const { name, re } of PATTERNS) {
      const m = text.match(re);
      if (m) {
        const line = text.slice(0, m.index).split('\n').length;
        findings.push({ file: relPath, line, kind: 'literal', detail: name });
      }
    }
  }

  if (relPath.startsWith('crates/') && relPath.endsWith('.rs')) {
    for (const match of text.matchAll(WRITE_CALLS)) {
      const openParen = match.index + match[0].length - 1;
      const args = extractCallArgs(text, openParen);
      if (args === null) continue;
      if (SECRET_HANDLES.some((re) => re.test(args))) {
        const line = text.slice(0, match.index).split('\n').length;
        findings.push({
          file: relPath,
          line,
          kind: 'write',
          detail: 'fs write whose argument references SecretString / .expose()',
        });
      }
    }
  }
}

if (findings.length === 0) {
  console.log('lint-secrets: ok');
  process.exit(0);
}

for (const f of findings) {
  console.error(`${f.file}:${f.line}: ${f.kind}: ${f.detail}`);
}
console.error(`\nlint-secrets: ${findings.length} finding(s)`);
process.exit(1);
