import assert from "node:assert/strict";
import { copyFile, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { CatalogStore } from "../lib/store.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

async function fixture(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "hd2-catalog-store-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const store = new CatalogStore({
    dataRoot: directory,
    seedPath: path.join(root, "data", "seed-catalog.json"),
    privateKeyPath: path.join(directory, "keys", "private.pem"),
    publicKeyPath: path.join(directory, "keys", "public.pem"),
  });
  await store.initialize();
  return { store, directory };
}

function reopenStore(directory) {
  return new CatalogStore({
    dataRoot: directory,
    seedPath: path.join(root, "data", "seed-catalog.json"),
    privateKeyPath: path.join(directory, "keys", "private.pem"),
    publicKeyPath: path.join(directory, "keys", "public.pem"),
  });
}

test("store initializes, signs and verifies the seed snapshot", async (t) => {
  const { store, directory } = await fixture(t);
  const { manifest, catalog } = await store.current();
  assert.equal(manifest.catalogVersion, 1);
  assert.equal(catalog.items.length, 101);
  assert.equal(await store.verifyCurrent(), true);
  assert.match(await readFile(path.join(directory, "keys", "public.pem"), "utf8"), /PUBLIC KEY/);
});

test("publication is serialized and stale writers are rejected", async (t) => {
  const { store } = await fixture(t);
  const { catalog } = await store.current();
  const first = store.publish({ baseVersion: 1, items: catalog.items, actor: "one@example.com" });
  const stale = store.publish({ baseVersion: 1, items: catalog.items, actor: "two@example.com" });
  assert.equal((await first).manifest.catalogVersion, 2);
  await assert.rejects(stale, (error) => error.code === "VERSION_CONFLICT" && error.currentVersion === 2);
});

test("rollback creates a new immutable version", async (t) => {
  const { store } = await fixture(t);
  const { catalog } = await store.current();
  const changed = structuredClone(catalog.items);
  changed[0].name.zh = "已修改";
  await store.publish({ baseVersion: 1, items: changed, actor: "admin@example.com" });
  const result = await store.rollback({ baseVersion: 2, targetVersion: 1, actor: "admin@example.com" });
  assert.equal(result.manifest.catalogVersion, 3);
  assert.notEqual(result.catalog.items[0].name.zh, "已修改");
  assert.equal((await store.version(2)).catalog.items[0].name.zh, "已修改");
  assert.deepEqual((await store.history()).map((entry) => entry.version), [3, 2, 1]);
});

test("existing entries must be disabled instead of deleted", async (t) => {
  const { store } = await fixture(t);
  const { catalog } = await store.current();
  await assert.rejects(
    store.publish({ baseVersion: 1, items: catalog.items.slice(1), actor: "admin@example.com" }),
    /must be disabled/,
  );
  assert.equal((await store.currentManifest()).catalogVersion, 1);
});

test("startup completes an audit entry committed before interruption", async (t) => {
  const { store, directory } = await fixture(t);
  const { catalog } = await store.current();
  await store.publish({ baseVersion: 1, items: catalog.items, actor: "admin@example.com" });
  const auditPath = path.join(directory, "audit.ndjson");
  const events = (await readFile(auditPath, "utf8")).trim().split("\n").map(JSON.parse);
  await writeFile(auditPath, `${JSON.stringify(events[0])}\n`);
  await writeFile(path.join(directory, "audit.pending.json"), `${JSON.stringify(events[1])}\n`);

  const reopened = reopenStore(directory);
  await reopened.initialize();
  assert.deepEqual((await reopened.history()).map((entry) => entry.version), [2, 1]);
  await assert.rejects(readFile(path.join(directory, "audit.pending.json")), /ENOENT/);
});

test("an interrupted uncommitted snapshot cannot block the next publication", async (t) => {
  const { store, directory } = await fixture(t);
  await copyFile(
    path.join(directory, "versions", "0000000001.json"),
    path.join(directory, "versions", "0000000002.json"),
  );
  await writeFile(
    path.join(directory, "audit.pending.json"),
    `${JSON.stringify({ version: 2, actor: "interrupted" })}\n`,
  );

  const reopened = reopenStore(directory);
  await reopened.initialize();
  const { catalog } = await reopened.current();
  const result = await reopened.publish({
    baseVersion: 1,
    items: catalog.items,
    actor: "admin@example.com",
  });
  assert.equal(result.manifest.catalogVersion, 3);
  assert.deepEqual((await reopened.history()).map((entry) => entry.version), [3, 1]);
});
