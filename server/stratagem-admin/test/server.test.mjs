import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { CatalogStore } from "../lib/store.mjs";
import { createCatalogServer } from "../server.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

async function fixture(t) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "hd2-catalog-server-"));
  const store = new CatalogStore({
    dataRoot: directory,
    seedPath: path.join(root, "data", "seed-catalog.json"),
    privateKeyPath: path.join(directory, "keys", "private.pem"),
    publicKeyPath: path.join(directory, "keys", "public.pem"),
  });
  await store.initialize();
  const server = createCatalogServer({
    store,
    access: { authenticate: async () => ({ email: "owner@example.com" }) },
    bundledIconRoot: path.resolve(root, "../../ui"),
    publicOrigin: "https://update.example.test",
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  t.after(async () => {
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });
  return { base: `http://127.0.0.1:${address.port}`, store };
}

test("public manifest and immutable catalog are available", async (t) => {
  const { base } = await fixture(t);
  const manifestResponse = await fetch(`${base}/api/v1/stratagems/manifest`);
  const manifest = await manifestResponse.json();
  assert.equal(manifest.catalogVersion, 1);
  assert.match(manifestResponse.headers.get("cache-control"), /max-age=60/);
  assert.equal(manifestResponse.headers.get("x-content-type-options"), "nosniff");
  const catalogResponse = await fetch(`${base}${manifest.catalogPath}`);
  assert.equal((await catalogResponse.json()).items.length, 101);
  assert.match(catalogResponse.headers.get("cache-control"), /immutable/);
});

test("admin publishes with CSRF checks and optimistic concurrency", async (t) => {
  const { base } = await fixture(t);
  const current = await (await fetch(`${base}/admin/api/catalog`)).json();
  const denied = await fetch(`${base}/admin/api/catalog`, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ baseVersion: 1, items: current.catalog.items }),
  });
  assert.equal(denied.status, 403);
  const published = await fetch(`${base}/admin/api/catalog`, {
    method: "PUT",
    headers: {
      "content-type": "application/json",
      "x-hd2-admin": "1",
      origin: "https://update.example.test",
    },
    body: JSON.stringify({ baseVersion: 1, items: current.catalog.items }),
  });
  assert.equal(published.status, 200);
  assert.equal((await published.json()).manifest.catalogVersion, 2);
});

test("admin serves only allowlisted assets and safe bundled icon names", async (t) => {
  const { base } = await fixture(t);
  assert.equal((await fetch(`${base}/admin/`)).status, 200);
  assert.equal((await fetch(`${base}/admin/app.js`)).status, 200);
  const stylesheet = await (await fetch(`${base}/admin/styles.css`)).text();
  assert.match(stylesheet, /\.catalog-pane[^}]*min-height:\s*0/s);
  assert.match(stylesheet, /\.catalog-list\s*{[^}]*min-height:\s*0[^}]*overflow-y:\s*auto/s);
  assert.equal((await fetch(`${base}/admin/secret.txt`)).status, 404);
  assert.equal((await fetch(`${base}/admin/api/bundled-icon/..%2Fsecret.svg`)).status, 404);
  assert.equal((await fetch(`${base}/admin/api/bundled-icon/not-present.svg`)).status, 404);
});

test("admin can be fail-closed while public catalog remains available", async (t) => {
  const { base, store } = await fixture(t);
  const server = createCatalogServer({
    store,
    access: { authenticate: async () => { throw new Error("must not run"); } },
    bundledIconRoot: path.resolve(root, "../../ui"),
    publicOrigin: "https://update.example.test",
    adminDisabled: true,
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const port = server.address().port;
  assert.equal((await fetch(`http://127.0.0.1:${port}/admin/`)).status, 503);
  assert.equal((await fetch(`http://127.0.0.1:${port}/api/v1/stratagems/manifest`)).status, 200);
});
