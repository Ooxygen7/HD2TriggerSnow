import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { normalizeItem, normalizeItems } from "../lib/catalog.mjs";
import { normalizeUploadedIcon } from "../lib/icons.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("seed catalog contains valid normalized items", async () => {
  const seed = JSON.parse(await readFile(path.join(root, "data", "seed-catalog.json"), "utf8"));
  const items = normalizeItems(seed.items);
  assert.equal(items.length, 101);
  assert.equal(items.some((item) => item.aliases.some((alias) => typeof alias !== "string")), false);
});

test("uploaded SVG is rasterized into the fixed safe wrapper", async () => {
  const source = Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><script>alert(1)</script><rect width="32" height="32" fill="#fff"/></svg>');
  const icon = await normalizeUploadedIcon({ mediaType: "image/svg+xml", base64: source.toString("base64") });
  const output = Buffer.from(icon.base64, "base64").toString("utf8");
  assert.match(output, /data-hd2-normalized-icon="1"/);
  assert.match(output, /data:image\/png;base64,/);
  assert.doesNotMatch(output, /<script|alert\(/i);
});

test("catalog rejects reserved IDs and invalid sequences", () => {
  const item = {
    id: "custom_forbidden",
    grp: "support",
    name: { zh: "测试", en: "Test" },
    aliases: [],
    ocr: [],
    seq: ["W", "X"],
    icon: { kind: "bundled", value: "Test.svg" },
    enabled: true,
    order: 1,
  };
  assert.throws(() => normalizeItem(item), /custom_/);
  assert.throws(() => normalizeItem({ ...item, id: "valid_id" }), /W, A, S, and D/);
});
