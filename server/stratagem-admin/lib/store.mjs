import { existsSync } from "node:fs";
import {
  chmod,
  mkdir,
  open,
  readFile,
  rename,
  stat,
  unlink,
} from "node:fs/promises";
import { createHash, createPrivateKey, createPublicKey, sign, verify } from "node:crypto";
import path from "node:path";
import {
  LIMITS,
  SCHEMA_VERSION,
  SIGNING_ALGORITHM,
  SIGNING_KEY_ID,
} from "./constants.mjs";
import { createCatalog, normalizeItems, serializeCatalog } from "./catalog.mjs";

function versionFilename(version) {
  return `${String(version).padStart(10, "0")}.json`;
}

function compactActor(actor) {
  return String(actor || "unknown").replace(/[\r\n\t]/g, " ").slice(0, 200);
}

async function syncDirectory(directory) {
  const handle = await open(directory, "r");
  try {
    try {
      await handle.sync();
    } catch (error) {
      if (process.platform !== "win32" || error.code !== "EPERM") throw error;
    }
  } finally {
    await handle.close();
  }
}

async function atomicWrite(filename, bytes, mode = 0o640) {
  const pending = `${filename}.pending`;
  const handle = await open(pending, "w", mode);
  try {
    await handle.writeFile(bytes);
    await handle.sync();
  } finally {
    await handle.close();
  }
  await chmod(pending, mode);
  await rename(pending, filename);
  await syncDirectory(path.dirname(filename));
}

async function appendAndSync(filename, bytes, mode = 0o640) {
  const handle = await open(filename, "a", mode);
  try {
    await handle.writeFile(bytes);
    await handle.sync();
  } finally {
    await handle.close();
  }
}

export async function generateSigningKeyPair(privateKeyPath, publicKeyPath) {
  const { generateKeyPairSync } = await import("node:crypto");
  const { privateKey, publicKey } = generateKeyPairSync("ed25519");
  await mkdir(path.dirname(privateKeyPath), { recursive: true, mode: 0o700 });
  await atomicWrite(privateKeyPath, privateKey.export({ type: "pkcs8", format: "pem" }), 0o600);
  await atomicWrite(publicKeyPath, publicKey.export({ type: "spki", format: "pem" }), 0o644);
}

export class CatalogStore {
  constructor({ dataRoot, seedPath, privateKeyPath, publicKeyPath }) {
    this.dataRoot = path.resolve(dataRoot);
    this.seedPath = path.resolve(seedPath);
    this.privateKeyPath = path.resolve(privateKeyPath);
    this.publicKeyPath = path.resolve(publicKeyPath);
    this.versionsRoot = path.join(this.dataRoot, "versions");
    this.manifestPath = path.join(this.dataRoot, "manifest.json");
    this.auditPath = path.join(this.dataRoot, "audit.ndjson");
    this.pendingAuditPath = path.join(this.dataRoot, "audit.pending.json");
    this.writeQueue = Promise.resolve();
    this.privateKey = null;
    this.publicKey = null;
  }

  async initialize() {
    await mkdir(this.versionsRoot, { recursive: true, mode: 0o750 });
    if (!existsSync(this.privateKeyPath) || !existsSync(this.publicKeyPath)) {
      await generateSigningKeyPair(this.privateKeyPath, this.publicKeyPath);
    }
    this.privateKey = createPrivateKey(await readFile(this.privateKeyPath));
    this.publicKey = createPublicKey(await readFile(this.publicKeyPath));
    if (!existsSync(this.manifestPath)) {
      const seed = JSON.parse(await readFile(this.seedPath, "utf8"));
      await this.publish({
        baseVersion: 0,
        items: seed.items,
        actor: "system",
        action: "initialize",
      });
    } else {
      await this.verifyCurrent();
      await this.#repairAudit();
    }
    return this.current();
  }

  publicKeyPem() {
    return this.publicKey.export({ type: "spki", format: "pem" }).toString();
  }

  async currentManifest() {
    return JSON.parse(await readFile(this.manifestPath, "utf8"));
  }

  async current() {
    const manifest = await this.currentManifest();
    const bytes = await readFile(path.join(this.versionsRoot, versionFilename(manifest.catalogVersion)));
    return { manifest, catalog: JSON.parse(bytes.toString("utf8")), bytes };
  }

  async version(version) {
    if (!Number.isSafeInteger(version) || version < 1) throw new TypeError("version is invalid");
    const filename = path.join(this.versionsRoot, versionFilename(version));
    const bytes = await readFile(filename);
    return { catalog: JSON.parse(bytes.toString("utf8")), bytes, filename };
  }

  async verifyCurrent() {
    const { manifest, catalog, bytes } = await this.current();
    if (catalog.catalogVersion !== manifest.catalogVersion) throw new Error("catalog version does not match manifest");
    const digest = createHash("sha256").update(bytes).digest("hex");
    if (digest !== manifest.sha256) throw new Error("catalog digest does not match manifest");
    if (!verify(null, bytes, this.publicKey, Buffer.from(manifest.signature, "base64"))) {
      throw new Error("catalog signature is invalid");
    }
    normalizeItems(catalog.items);
    return true;
  }

  publish(input) {
    const operation = this.writeQueue.then(() => this.#publishNow(input));
    this.writeQueue = operation.catch(() => {});
    return operation;
  }

  async #auditEvents() {
    if (!existsSync(this.auditPath)) return [];
    const lines = (await readFile(this.auditPath, "utf8")).trim().split("\n").filter(Boolean);
    return lines.map((line) => JSON.parse(line));
  }

  async #repairAudit() {
    const manifest = await this.currentManifest();
    const events = await this.#auditEvents();
    const recordedVersions = new Set(events.map((event) => Number(event.version)));
    let pending = null;
    if (existsSync(this.pendingAuditPath)) {
      pending = JSON.parse(await readFile(this.pendingAuditPath, "utf8"));
    }

    if (
      pending
      && Number.isSafeInteger(pending.version)
      && pending.version <= manifest.catalogVersion
      && !recordedVersions.has(pending.version)
    ) {
      await appendAndSync(this.auditPath, `${JSON.stringify(pending)}\n`);
      recordedVersions.add(pending.version);
    }

    if (!recordedVersions.has(manifest.catalogVersion)) {
      const recovered = {
        version: manifest.catalogVersion,
        publishedAt: manifest.publishedAt,
        actor: "system",
        action: "recovered",
        sourceVersion: null,
        itemCount: manifest.itemCount,
        sha256: manifest.sha256,
      };
      await appendAndSync(this.auditPath, `${JSON.stringify(recovered)}\n`);
    }

    if (existsSync(this.pendingAuditPath)) {
      await unlink(this.pendingAuditPath);
      await syncDirectory(this.dataRoot);
    }
  }

  async #publishNow({ baseVersion, items, actor, action = "publish", sourceVersion = null }) {
    const currentVersion = existsSync(this.manifestPath)
      ? (await this.currentManifest()).catalogVersion
      : 0;
    if (Number(baseVersion) !== currentVersion) {
      const error = new Error(`catalog changed from version ${baseVersion} to ${currentVersion}`);
      error.code = "VERSION_CONFLICT";
      error.currentVersion = currentVersion;
      throw error;
    }

    let version = currentVersion + 1;
    while (existsSync(path.join(this.versionsRoot, versionFilename(version)))) version += 1;
    const publishedAt = new Date().toISOString();
    const catalog = createCatalog({ version, publishedAt, items });
    if (currentVersion > 0) {
      const { catalog: currentCatalog } = await this.current();
      const nextIds = new Set(catalog.items.map((item) => item.id.toLocaleLowerCase()));
      const removed = currentCatalog.items.find((item) => !nextIds.has(item.id.toLocaleLowerCase()));
      if (removed) {
        throw new TypeError(`stratagem ${removed.id} must be disabled instead of removed`);
      }
    }
    const bytes = serializeCatalog(catalog);
    const sha256 = createHash("sha256").update(bytes).digest("hex");
    const signature = sign(null, bytes, this.privateKey).toString("base64");
    const manifest = {
      schemaVersion: SCHEMA_VERSION,
      catalogVersion: version,
      publishedAt,
      minAppVersion: catalog.minAppVersion,
      itemCount: catalog.items.length,
      sha256,
      signature,
      signingAlgorithm: SIGNING_ALGORITHM,
      keyId: SIGNING_KEY_ID,
      catalogPath: `/api/v1/stratagems/catalog/${version}`,
    };

    const versionPath = path.join(this.versionsRoot, versionFilename(version));
    try {
      await stat(versionPath);
      throw new Error(`catalog version ${version} already exists`);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    const event = {
      version,
      publishedAt,
      actor: compactActor(actor),
      action: String(action).slice(0, 40),
      sourceVersion: sourceVersion == null ? null : Number(sourceVersion),
      itemCount: catalog.items.length,
      sha256,
    };
    await atomicWrite(versionPath, bytes, 0o640);
    await atomicWrite(this.pendingAuditPath, Buffer.from(`${JSON.stringify(event)}\n`), 0o640);
    await atomicWrite(this.manifestPath, Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`), 0o640);
    await appendAndSync(this.auditPath, `${JSON.stringify(event)}\n`);
    await unlink(this.pendingAuditPath);
    await syncDirectory(this.dataRoot);
    return { manifest, catalog };
  }

  async rollback({ baseVersion, targetVersion, actor }) {
    const { catalog } = await this.version(Number(targetVersion));
    return this.publish({
      baseVersion,
      items: catalog.items,
      actor,
      action: "rollback",
      sourceVersion: targetVersion,
    });
  }

  async history(limit = 100) {
    const bounded = Math.max(1, Math.min(Number(limit) || 100, LIMITS.historyEntries));
    const events = await this.#auditEvents();
    return events.slice(-bounded).reverse();
  }
}
