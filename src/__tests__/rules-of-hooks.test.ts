import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import ts from "typescript";

/**
 * No hook may be called after an early return.
 *
 * This has now shipped twice, both times in FirstRunSetup, and both times the
 * symptom was the whole app going blank with React error #310 ("rendered more
 * hooks than during the previous render"). It hides from everyday use because
 * it only fires when the guard flips — so an onboarding screen that an existing
 * booth never opens was broken on EVERY fresh install for four releases before
 * anyone installed ProDeck somewhere new.
 *
 * eslint-plugin-react-hooks would catch it, but it isn't wired into this repo's
 * test run; this is the check that actually gates a release.
 */

function sourceFiles(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) sourceFiles(p, out);
    else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name)) out.push(p);
  }
  return out;
}

const isHookCall = (n: ts.Node): n is ts.CallExpression =>
  ts.isCallExpression(n) &&
  ts.isIdentifier(n.expression) &&
  /^use[A-Z]/.test(n.expression.text);

/** Hook calls in this subtree, not descending into nested function bodies. */
function hooksIn(node: ts.Node, found: ts.CallExpression[] = []): ts.CallExpression[] {
  node.forEachChild((child) => {
    if (
      ts.isFunctionDeclaration(child) ||
      ts.isFunctionExpression(child) ||
      ts.isArrowFunction(child) ||
      ts.isMethodDeclaration(child)
    ) {
      return; // a hook in a callback is a different (also invalid) shape
    }
    if (isHookCall(child)) found.push(child);
    hooksIn(child, found);
  });
  return found;
}

function violations(file: string): string[] {
  const text = readFileSync(file, "utf8");
  const sf = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true);
  const out: string[] = [];

  const checkBody = (name: string, body: ts.Block) => {
    // Components and hooks only — a plain helper may return early freely.
    if (!/^(use[A-Z]|[A-Z])/.test(name)) return;
    let returnedAt: ts.Statement | null = null;
    for (const stmt of body.statements) {
      if (returnedAt) {
        for (const hook of hooksIn(stmt)) {
          const { line } = sf.getLineAndCharacterOfPosition(hook.getStart(sf));
          const call = (hook.expression as ts.Identifier).text;
          out.push(`${file}:${line + 1} — ${call}() in ${name}() after an early return`);
        }
        continue;
      }
      if (ts.isReturnStatement(stmt)) returnedAt = stmt;
      if (ts.isIfStatement(stmt) && stmt.thenStatement) {
        const t = stmt.thenStatement;
        const returnsInside =
          ts.isReturnStatement(t) ||
          (ts.isBlock(t) && t.statements.some((s) => ts.isReturnStatement(s)));
        if (returnsInside && !stmt.elseStatement) returnedAt = stmt;
      }
    }
  };

  const walk = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name && node.body) {
      checkBody(node.name.text, node.body);
    }
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
      const init = node.initializer;
      if ((ts.isArrowFunction(init) || ts.isFunctionExpression(init)) && init.body && ts.isBlock(init.body)) {
        checkBody(node.name.text, init.body);
      }
    }
    node.forEachChild(walk);
  };
  walk(sf);
  return out;
}

describe("rules of hooks", () => {
  const files = sourceFiles("src");

  it("scans a meaningful number of files (guards against a silent no-op)", () => {
    expect(files.length).toBeGreaterThan(40);
  });

  it("calls no hook after an early return", () => {
    const all = files.flatMap(violations);
    expect(all, `\n${all.join("\n")}\n`).toEqual([]);
  });
});
