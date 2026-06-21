import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const source = fs.readFileSync(path.join(root, "ui", "index.html"), "utf8");
const start = source.indexOf("function normalizeOcrText(value)");
const end = source.indexOf("let isMacroRunning", start);

assert.notEqual(start, -1, "could not find the OCR matcher in index.html");
assert.notEqual(end, -1, "could not find the OCR matcher end marker");

const matcherSource = source.slice(start, end);

function matcherContext({ stratagems, maxSlots = 4, loadout = [] }) {
  const context = vm.createContext({ console });
  vm.runInContext(
    `
      let stratagemDB = ${JSON.stringify(stratagems)};
      let maxSlots = ${maxSlots};
      let activeLoadout = ${JSON.stringify(loadout)};
      let overlaySelectedIndex = -1;
      let committed = false;
      function commitLoadoutChange() { committed = true; }
      ${matcherSource}
    `,
    context,
    { filename: "index.html:ocr-matcher" },
  );
  return context;
}

function evaluate(context, expression) {
  return vm.runInContext(expression, context, { filename: "ocr-matcher-regression" });
}

// Intentional legacy rule 1: a partial term never falls through to fuzzy matching.
{
  const context = matcherContext({
    stratagems: [{ id: "alpha", grp: "support", ocr: ["Alpha"] }],
  });
  assert.deepEqual(
    JSON.parse(evaluate(context, "JSON.stringify(matchOcrStratagems('Alph').map(item => item.id))")),
    [],
  );
}

// Intentional legacy rule 2: exact lines are removed before fuzzy scores are indexed.
{
  const context = matcherContext({
    stratagems: [
      { id: "alpha", grp: "support", ocr: ["Alpha"] },
      { id: "beta", grp: "support", ocr: ["Bettor"] },
    ],
  });
  evaluate(
    context,
    `
      const originalScore = scoreStratagemAgainstOcr;
      globalThis.capturedFuzzyLines = [];
      scoreStratagemAgainstOcr = (entry, lines) => {
        capturedFuzzyLines.push([...lines]);
        return originalScore(entry, lines);
      };
      matchOcrStratagems('Alpha\\nBettar');
    `,
  );
  assert.deepEqual(
    JSON.parse(evaluate(context, "JSON.stringify(capturedFuzzyLines[1])")),
    ["bettar"],
  );
  assert.equal(
    evaluate(
      context,
      "scoreStratagemAgainstOcr(getOcrEntries().find(entry => entry.strat.id === 'beta'), ['bettar']).firstIndex",
    ),
    0,
  );
}

// Intentional legacy rule 3: clear unlocked slots before returning no-space.
{
  const context = matcherContext({
    stratagems: [],
    maxSlots: 2,
    loadout: [
      { locked: false, strat: { id: "old-unlocked" } },
      { locked: true, strat: { id: "locked" } },
    ],
  });
  const result = JSON.parse(
    evaluate(
      context,
      "JSON.stringify(applyOcrLoadout([{ id: 'new-a' }, { id: 'new-b' }]))",
    ),
  );
  assert.deepEqual(result, { ok: false, reason: "no-space", count: 0 });
  assert.equal(evaluate(context, "activeLoadout[0].strat"), null);
  assert.equal(evaluate(context, "activeLoadout[1].strat.id"), "locked");
}

console.log("OCR matcher legacy-regression tests passed.");
