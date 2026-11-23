#!/usr/bin/env node
/**
 * Probe: how long is one edit-to-verdict cycle in TypeScript versus Rust?
 *
 * An iterative workflow pays this cost on every iteration, so it is the dominant
 * cost of the development method, not a detail of the stack. Answers a question
 * that has a measurable answer, so it gets measured (stele:ADR-0004).
 *
 * Scaffolds an equivalent trivial library in each stack: one pure function, one
 * passing test, one deliberately failing variant. Measures:
 *   - setup:  getting from empty directory to a runnable project
 *   - cold:   first full check-and-test, nothing cached
 *   - warm:   the same after a one-line edit, caches hot. This is the loop cost.
 *
 * Run: node scripts/stack-cycle-probe.mjs <workdir>
 */
import { execFileSync } from 'node:child_process';
import { mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';

const workdir = process.argv[2];
if (!workdir) {
  console.error('usage: node scripts/stack-cycle-probe.mjs <workdir>');
  process.exit(1);
}

/** Wall-clock milliseconds for one shell command, or null if it failed. */
function time(label, command, args, cwd) {
  const startedAt = process.hrtime.bigint();
  try {
    execFileSync(command, args, { cwd, stdio: 'pipe' });
  } catch (error) {
    const elapsed = Number(process.hrtime.bigint() - startedAt) / 1e6;
    const output = `${String(error.stdout ?? '')}${String(error.stderr ?? '')}`.trim();
    return { label, ms: Math.round(elapsed), failed: true, output: output.slice(-600) };
  }
  const elapsed = Number(process.hrtime.bigint() - startedAt) / 1e6;
  return { label, ms: Math.round(elapsed), failed: false };
}

rmSync(workdir, { recursive: true, force: true });
mkdirSync(workdir, { recursive: true });

const results = [];

// ---------- TypeScript ----------
const ts = join(workdir, 'ts');
mkdirSync(join(ts, 'src'), { recursive: true });
writeFileSync(join(ts, 'package.json'), JSON.stringify({
  name: 'probe-ts', private: true, type: 'module',
  scripts: { check: 'tsc --noEmit', test: 'vitest run' },
  devDependencies: { typescript: '5.9.2', vitest: '3.2.4', '@types/node': '22.18.1' },
}, null, 2));
writeFileSync(join(ts, 'tsconfig.json'), JSON.stringify({
  compilerOptions: {
    strict: true, target: 'ES2023', lib: ['ESNext'], module: 'ESNext',
    moduleResolution: 'bundler', noEmit: true, skipLibCheck: true, types: ['node'],
  },
  include: ['src'],
}, null, 2));
writeFileSync(join(ts, 'src/sum.ts'), 'export const sum = (a: number, b: number): number => a + b;\n');
writeFileSync(join(ts, 'src/sum.test.ts'), [
  "import { expect, test } from 'vitest';",
  "import { sum } from './sum.js';",
  "test('sums two numbers', () => { expect(sum(2, 3)).toBe(5); });",
  '',
].join('\n'));

results.push(time('ts:setup (npm install)', 'npm', ['install', '--silent', '--no-audit', '--no-fund'], ts));
results.push(time('ts:cold check', 'npx', ['tsc', '--noEmit'], ts));
results.push(time('ts:cold test', 'npx', ['vitest', 'run'], ts));
writeFileSync(join(ts, 'src/sum.ts'), 'export const sum = (a: number, b: number): number => a + b + 0;\n');
results.push(time('ts:warm check', 'npx', ['tsc', '--noEmit'], ts));
results.push(time('ts:warm test', 'npx', ['vitest', 'run'], ts));

// ---------- Rust ----------
const rs = join(workdir, 'rs');
mkdirSync(join(rs, 'src'), { recursive: true });
writeFileSync(join(rs, 'Cargo.toml'), '[package]\nname = "probe_rs"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n');
const rustSource = (extra) => [
  'pub fn sum(a: i64, b: i64) -> i64 {',
  `    a + b${extra}`,
  '}',
  '',
  '#[cfg(test)]',
  'mod tests {',
  '    use super::*;',
  '    #[test]',
  '    fn sums_two_numbers() { assert_eq!(sum(2, 3), 5); }',
  '}',
  '',
].join('\n');
writeFileSync(join(rs, 'src/lib.rs'), rustSource(''));

results.push(time('rs:setup (cargo fetch)', 'cargo', ['fetch'], rs));
results.push(time('rs:cold check', 'cargo', ['check', '--quiet'], rs));
results.push(time('rs:cold test', 'cargo', ['test', '--quiet'], rs));
writeFileSync(join(rs, 'src/lib.rs'), rustSource(' + 0'));
results.push(time('rs:warm check', 'cargo', ['check', '--quiet'], rs));
results.push(time('rs:warm test', 'cargo', ['test', '--quiet'], rs));

// ---------- Phase 2: a realistic dependency graph ----------
// The trivial case is Rust's best case: a crate with no dependencies. The project
// we are actually choosing a stack for needs an async runtime, an HTTP stack, a
// GitHub client and serialization. Measuring without them would flatter Rust.

const tsDeps = join(workdir, 'ts-deps');
mkdirSync(join(tsDeps, 'src'), { recursive: true });
writeFileSync(join(tsDeps, 'package.json'), JSON.stringify({
  name: 'probe-ts-deps', private: true, type: 'module',
  devDependencies: { typescript: '5.9.2', vitest: '3.2.4', '@types/node': '22.18.1' },
  dependencies: { '@octokit/rest': '21.1.1', zod: '3.25.76' },
}, null, 2));
writeFileSync(join(tsDeps, 'tsconfig.json'), JSON.stringify({
  compilerOptions: {
    strict: true, target: 'ES2023', lib: ['ESNext'], module: 'ESNext',
    moduleResolution: 'bundler', noEmit: true, skipLibCheck: true, types: ['node'],
  },
  include: ['src'],
}, null, 2));
const tsDepsSource = (extra) => [
  "import { Octokit } from '@octokit/rest';",
  "import { z } from 'zod';",
  'export const pullNumber = z.object({ number: z.number() });',
  'export const client = (token: string): Octokit => new Octokit({ auth: token });',
  `export const sum = (a: number, b: number): number => a + b${extra};`,
  '',
].join('\n');
writeFileSync(join(tsDeps, 'src/client.ts'), tsDepsSource(''));

results.push(time('ts+deps:setup (npm install)', 'npm', ['install', '--silent', '--no-audit', '--no-fund'], tsDeps));
results.push(time('ts+deps:cold check', 'npx', ['tsc', '--noEmit'], tsDeps));
writeFileSync(join(tsDeps, 'src/client.ts'), tsDepsSource(' + 0'));
results.push(time('ts+deps:warm check', 'npx', ['tsc', '--noEmit'], tsDeps));

const rsDeps = join(workdir, 'rs-deps');
mkdirSync(join(rsDeps, 'src'), { recursive: true });
writeFileSync(join(rsDeps, 'Cargo.toml'), [
  '[package]', 'name = "probe_rs_deps"', 'version = "0.1.0"', 'edition = "2021"', '',
  '[dependencies]',
  'tokio = { version = "1.47.1", features = ["full"] }',
  'octocrab = "0.44.1"',
  'serde = { version = "1.0.219", features = ["derive"] }',
  'serde_json = "1.0.145"',
  '',
].join('\n'));
const rsDepsSource = (extra) => [
  'use serde::Deserialize;',
  '',
  '#[derive(Deserialize)]',
  'pub struct PullNumber { pub number: u64 }',
  '',
  'pub async fn client(token: String) -> Result<octocrab::Octocrab, octocrab::Error> {',
  '    octocrab::Octocrab::builder().personal_token(token).build()',
  '}',
  '',
  `pub fn sum(a: i64, b: i64) -> i64 { a + b${extra} }`,
  '',
].join('\n');
writeFileSync(join(rsDeps, 'src/lib.rs'), rsDepsSource(''));

results.push(time('rs+deps:setup (cargo fetch)', 'cargo', ['fetch'], rsDeps));
results.push(time('rs+deps:cold check', 'cargo', ['check', '--quiet'], rsDeps));
writeFileSync(join(rsDeps, 'src/lib.rs'), rsDepsSource(' + 0'));
results.push(time('rs+deps:warm check', 'cargo', ['check', '--quiet'], rsDeps));

const width = Math.max(...results.map((r) => r.label.length));
for (const r of results) {
  console.log(`${r.label.padEnd(width)}  ${String(r.ms).padStart(7)} ms${r.failed ? '   FAILED' : ''}`);
  if (r.failed) console.log(`${' '.repeat(width)}  ${r.output.replace(/\n/g, '\n' + ' '.repeat(width + 2))}`);
}

const total = (prefix, stage) =>
  results.filter((r) => r.label.startsWith(`${prefix}:${stage}`)).reduce((sum, r) => sum + r.ms, 0);

console.log('\n--- trivial project (no dependencies) ---');
console.log(`warm cycle (check + test): ts=${total('ts', 'warm')} ms  rs=${total('rs', 'warm')} ms`);
console.log('\n--- realistic dependency graph ---');
console.log(`cold check: ts=${total('ts+deps', 'cold')} ms  rs=${total('rs+deps', 'cold')} ms`);
console.log(`warm check: ts=${total('ts+deps', 'warm')} ms  rs=${total('rs+deps', 'warm')} ms`);
