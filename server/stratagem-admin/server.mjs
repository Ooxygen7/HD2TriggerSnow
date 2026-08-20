import { createServer } from "node:http";
import { readFile, stat } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { AccessVerifier } from "./lib/access.mjs";
import { BUNDLED_ICON_PATTERN, LIMITS } from "./lib/constants.mjs";
import { normalizeUploadedIcon } from "./lib/icons.mjs";
import { CatalogStore } from "./lib/store.mjs";

const root = path.dirname(fileURLToPath(import.meta.url));
const publicRoot = path.join(root, "public");

const MIME_TYPES = new Map([
  [".html", "text/html; charset=utf-8"],
  [".css", "text/css; charset=utf-8"],
  [".js", "text/javascript; charset=utf-8"],
  [".svg", "image/svg+xml"],
  [".png", "image/png"],
]);

function securityHeaders(response, cacheControl = "no-store") {
  response.setHeader("Cache-Control", cacheControl);
  response.setHeader("X-Content-Type-Options", "nosniff");
  response.setHeader("Referrer-Policy", "no-referrer");
  response.setHeader("X-Frame-Options", "DENY");
  response.setHeader("Permissions-Policy", "camera=(), microphone=(), geolocation=()");
  response.setHeader(
    "Content-Security-Policy",
    "default-src 'none'; img-src 'self' data:; style-src 'self'; script-src 'self'; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'",
  );
}

function json(response, status, value, cacheControl = "no-store") {
  securityHeaders(response, cacheControl);
  response.statusCode = status;
  response.setHeader("Content-Type", "application/json; charset=utf-8");
  response.end(`${JSON.stringify(value)}\n`);
}

function text(response, status, value) {
  securityHeaders(response);
  response.statusCode = status;
  response.setHeader("Content-Type", "text/plain; charset=utf-8");
  response.end(value);
}

async function readJsonBody(request, maximum = LIMITS.requestBytes) {
  const contentType = String(request.headers["content-type"] || "").split(";", 1)[0].trim();
  if (contentType !== "application/json") {
    const error = new Error("Content-Type must be application/json");
    error.statusCode = 415;
    throw error;
  }
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > maximum) {
      const error = new Error("request body is too large");
      error.statusCode = 413;
      throw error;
    }
    chunks.push(chunk);
  }
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    const error = new Error("request body is not valid JSON");
    error.statusCode = 400;
    throw error;
  }
}

function requireMutationHeaders(request, publicOrigin) {
  if (request.headers["x-hd2-admin"] !== "1") {
    const error = new Error("admin request header is missing");
    error.statusCode = 403;
    throw error;
  }
  const origin = String(request.headers.origin || "");
  if (origin !== publicOrigin) {
    const error = new Error("request origin is not allowed");
    error.statusCode = 403;
    throw error;
  }
}

function publicError(error) {
  if (error?.code === "VERSION_CONFLICT") {
    return { status: 409, body: { error: "version_conflict", currentVersion: error.currentVersion } };
  }
  const status = Number(error?.statusCode) || (error instanceof TypeError ? 400 : 500);
  return {
    status,
    body: {
      error: status >= 500 ? "internal_error" : "invalid_request",
      message: status >= 500 ? "The request could not be completed." : String(error.message).slice(0, 240),
    },
  };
}

async function sendFile(response, filename, cacheControl = "no-store") {
  const extension = path.extname(filename).toLowerCase();
  const bytes = await readFile(filename);
  securityHeaders(response, cacheControl);
  response.statusCode = 200;
  response.setHeader("Content-Type", MIME_TYPES.get(extension) || "application/octet-stream");
  response.setHeader("Content-Length", bytes.length);
  response.end(bytes);
}

function isMissingFile(error) {
  return error?.code === "ENOENT" || error?.code === "ENOTDIR";
}

export async function validateBundledIcons(store, bundledIconRoot) {
  const resolvedIcons = path.resolve(bundledIconRoot);
  const { catalog } = await store.current();
  const filenames = [...new Set(
    catalog.items
      .filter((item) => item.icon?.kind === "bundled")
      .map((item) => item.icon.value),
  )];
  const missing = [];
  for (const filename of filenames) {
    if (!BUNDLED_ICON_PATTERN.test(filename)) {
      missing.push(filename);
      continue;
    }
    const iconPath = path.resolve(resolvedIcons, filename);
    if (path.dirname(iconPath) !== resolvedIcons) {
      missing.push(filename);
      continue;
    }
    try {
      if (!(await stat(iconPath)).isFile()) missing.push(filename);
    } catch (error) {
      if (isMissingFile(error)) missing.push(filename);
      else throw error;
    }
  }
  if (missing.length > 0) {
    const preview = missing.slice(0, 5).join(", ");
    throw new Error(`Bundled icon validation failed (${missing.length} missing): ${preview}`);
  }
  return filenames.length;
}

export function createCatalogServer({ store, access, bundledIconRoot, publicOrigin, adminDisabled = false }) {
  const resolvedIcons = path.resolve(bundledIconRoot);
  return createServer(async (request, response) => {
    const requestUrl = new URL(request.url, publicOrigin);
    const pathname = requestUrl.pathname;
    try {
      if (request.method === "GET" && pathname === "/health") {
        return json(response, 200, { status: "ok", service: "hd2-stratagem-catalog" });
      }
      if (request.method === "GET" && pathname === "/api/v1/stratagems/manifest") {
        const manifest = await store.currentManifest();
        return json(response, 200, manifest, "public, max-age=60, stale-if-error=86400");
      }
      const catalogMatch = pathname.match(/^\/api\/v1\/stratagems\/catalog\/(\d{1,10})$/);
      if (request.method === "GET" && catalogMatch) {
        const { bytes } = await store.version(Number(catalogMatch[1]));
        securityHeaders(response, "public, max-age=31536000, immutable");
        response.statusCode = 200;
        response.setHeader("Content-Type", "application/json; charset=utf-8");
        response.setHeader("Content-Length", bytes.length);
        return response.end(bytes);
      }

      if (!pathname.startsWith("/admin")) return json(response, 404, { error: "not_found" });
      if (adminDisabled) return json(response, 503, { error: "admin_disabled" });
      let identity;
      try {
        identity = await access.authenticate(request);
      } catch {
        return json(response, 403, { error: "access_denied" });
      }

      if (request.method === "GET" && pathname === "/admin/api/session") {
        const manifest = await store.currentManifest();
        return json(response, 200, { user: identity.email, catalogVersion: manifest.catalogVersion });
      }
      if (request.method === "GET" && pathname === "/admin/api/catalog") {
        const { manifest, catalog } = await store.current();
        return json(response, 200, { manifest, catalog });
      }
      if (request.method === "GET" && pathname === "/admin/api/history") {
        return json(response, 200, { history: await store.history(requestUrl.searchParams.get("limit")) });
      }
      const bundledMatch = pathname.match(/^\/admin\/api\/bundled-icon\/([^/]+)$/);
      if (request.method === "GET" && bundledMatch) {
        const filename = decodeURIComponent(bundledMatch[1]);
        if (!BUNDLED_ICON_PATTERN.test(filename)) return json(response, 404, { error: "not_found" });
        const iconPath = path.resolve(resolvedIcons, filename);
        if (path.dirname(iconPath) !== resolvedIcons) return json(response, 404, { error: "not_found" });
        try {
          return await sendFile(response, iconPath, "private, max-age=3600");
        } catch (error) {
          if (isMissingFile(error)) return json(response, 404, { error: "not_found" });
          throw error;
        }
      }

      if (request.method === "POST" && pathname === "/admin/api/icons/normalize") {
        requireMutationHeaders(request, publicOrigin);
        const body = await readJsonBody(request, 2 * 1024 * 1024);
        const icon = await normalizeUploadedIcon(body);
        return json(response, 200, { icon });
      }
      if (request.method === "PUT" && pathname === "/admin/api/catalog") {
        requireMutationHeaders(request, publicOrigin);
        const body = await readJsonBody(request);
        const result = await store.publish({
          baseVersion: Number(body.baseVersion),
          items: body.items,
          actor: identity.email,
          action: "publish",
        });
        return json(response, 200, { manifest: result.manifest, catalog: result.catalog });
      }
      if (request.method === "POST" && pathname === "/admin/api/rollback") {
        requireMutationHeaders(request, publicOrigin);
        const body = await readJsonBody(request, 64 * 1024);
        const result = await store.rollback({
          baseVersion: Number(body.baseVersion),
          targetVersion: Number(body.targetVersion),
          actor: identity.email,
        });
        return json(response, 200, { manifest: result.manifest, catalog: result.catalog });
      }

      if (request.method !== "GET" && request.method !== "HEAD") {
        return json(response, 405, { error: "method_not_allowed" });
      }
      let asset = pathname === "/admin" || pathname === "/admin/"
        ? "index.html"
        : pathname.slice("/admin/".length);
      if (!/^(?:index\.html|styles\.css|app\.js)$/.test(asset)) {
        return json(response, 404, { error: "not_found" });
      }
      try {
        return await sendFile(response, path.join(publicRoot, asset), asset === "index.html" ? "no-store" : "private, max-age=3600");
      } catch (error) {
        if (isMissingFile(error)) return json(response, 404, { error: "not_found" });
        throw error;
      }
    } catch (error) {
      const { status, body } = publicError(error);
      if (status >= 500) console.error(error);
      return json(response, status, body);
    }
  });
}

async function main() {
  const host = process.env.HD2_CATALOG_HOST || "127.0.0.1";
  const port = Number(process.env.HD2_CATALOG_PORT || 8785);
  const dataRoot = process.env.HD2_CATALOG_DATA || path.join(root, ".data");
  const keyRoot = process.env.HD2_CATALOG_KEYS || path.join(dataRoot, "keys");
  const publicOrigin = process.env.HD2_PUBLIC_ORIGIN || `http://${host}:${port}`;
  const bundledIconRoot = process.env.HD2_BUNDLED_ICONS || path.join(root, "bundled-icons");
  const adminDisabled = process.env.HD2_ADMIN_DISABLED === "1";
  const store = new CatalogStore({
    dataRoot,
    seedPath: process.env.HD2_SEED_CATALOG || path.join(root, "data", "seed-catalog.json"),
    privateKeyPath: path.join(keyRoot, "catalog-signing-private.pem"),
    publicKeyPath: path.join(keyRoot, "catalog-signing-public.pem"),
  });
  await store.initialize();
  const bundledIconCount = await validateBundledIcons(store, bundledIconRoot);
  const access = new AccessVerifier({
    teamDomain: process.env.CF_ACCESS_TEAM_DOMAIN,
    audience: process.env.CF_ACCESS_AUD,
    allowedEmails: String(process.env.CF_ACCESS_ALLOWED_EMAILS || "").split(","),
    devBypass: process.env.HD2_ADMIN_DEV_BYPASS === "1" && host === "127.0.0.1",
  });
  if (!adminDisabled && !access.configured() && !access.devBypass) {
    throw new Error("Cloudflare Access requires a team domain, audience and exactly one allowed email");
  }
  const server = createCatalogServer({ store, access, bundledIconRoot, publicOrigin, adminDisabled });
  server.listen(port, host, () => {
    console.log(`HD2 stratagem catalog listening on http://${host}:${port} with ${bundledIconCount} bundled icons`);
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
