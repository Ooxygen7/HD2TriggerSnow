import { createPublicKey, verify } from "node:crypto";

function decodePart(value) {
  return JSON.parse(Buffer.from(value, "base64url").toString("utf8"));
}

function tokenAudienceIncludes(audience, expected) {
  return Array.isArray(audience) ? audience.includes(expected) : audience === expected;
}

function normalizeTeamDomain(value) {
  const domain = String(value || "").trim().replace(/\/+$/, "");
  if (!/^https:\/\/[a-z0-9.-]+\.cloudflareaccess\.com$/i.test(domain)) {
    throw new Error("CF_ACCESS_TEAM_DOMAIN is invalid");
  }
  return domain;
}

export class AccessVerifier {
  constructor({ teamDomain, audience, allowedEmails = [], devBypass = false, jwksProvider }) {
    this.teamDomain = teamDomain ? normalizeTeamDomain(teamDomain) : null;
    this.audience = String(audience || "").trim();
    this.allowedEmails = new Set(allowedEmails.map((value) => value.trim().toLowerCase()).filter(Boolean));
    this.devBypass = devBypass === true;
    this.jwksProvider = jwksProvider || (() => this.#remoteJwks());
    this.jwks = null;
    this.jwksExpiresAt = 0;
  }

  configured() {
    return Boolean(this.teamDomain && this.audience && this.allowedEmails.size === 1);
  }

  async #remoteJwks() {
    const response = await fetch(`${this.teamDomain}/cdn-cgi/access/certs`, {
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(4000),
    });
    if (!response.ok) throw new Error(`Cloudflare JWKS returned HTTP ${response.status}`);
    const body = await response.text();
    if (Buffer.byteLength(body) > 256 * 1024) throw new Error("Cloudflare JWKS response is too large");
    const parsed = JSON.parse(body);
    if (!Array.isArray(parsed.keys) || parsed.keys.length === 0) throw new Error("Cloudflare JWKS contains no keys");
    return parsed.keys;
  }

  async #keys(force = false) {
    if (!force && this.jwks && Date.now() < this.jwksExpiresAt) return this.jwks;
    this.jwks = await this.jwksProvider();
    this.jwksExpiresAt = Date.now() + 60 * 60 * 1000;
    return this.jwks;
  }

  async verifyToken(token) {
    if (!this.configured()) throw new Error("Cloudflare Access is not configured");
    if (typeof token !== "string" || token.length < 32 || token.length > 16384) {
      throw new Error("Cloudflare Access token is missing");
    }
    const parts = token.split(".");
    if (parts.length !== 3) throw new Error("Cloudflare Access token is malformed");
    const header = decodePart(parts[0]);
    const payload = decodePart(parts[1]);
    if (header.alg !== "RS256" || typeof header.kid !== "string") {
      throw new Error("Cloudflare Access token algorithm is unsupported");
    }
    let keys = await this.#keys();
    let jwk = keys.find((key) => key.kid === header.kid && key.kty === "RSA");
    if (!jwk) {
      keys = await this.#keys(true);
      jwk = keys.find((key) => key.kid === header.kid && key.kty === "RSA");
    }
    if (!jwk) throw new Error("Cloudflare Access signing key is unknown");
    const valid = verify(
      "RSA-SHA256",
      Buffer.from(`${parts[0]}.${parts[1]}`),
      createPublicKey({ key: jwk, format: "jwk" }),
      Buffer.from(parts[2], "base64url"),
    );
    if (!valid) throw new Error("Cloudflare Access token signature is invalid");
    const now = Math.floor(Date.now() / 1000);
    if (payload.iss !== this.teamDomain) throw new Error("Cloudflare Access token issuer is invalid");
    if (!tokenAudienceIncludes(payload.aud, this.audience)) throw new Error("Cloudflare Access token audience is invalid");
    if (!Number.isFinite(payload.exp) || payload.exp <= now) throw new Error("Cloudflare Access token has expired");
    if (Number.isFinite(payload.nbf) && payload.nbf > now + 30) throw new Error("Cloudflare Access token is not active");
    const email = typeof payload.email === "string" ? payload.email.trim().toLowerCase() : "";
    if (!email) throw new Error("Cloudflare Access token contains no email");
    if (this.allowedEmails.size > 0 && !this.allowedEmails.has(email)) {
      throw new Error("Cloudflare Access identity is not allowed");
    }
    return { email, subject: String(payload.sub || "") };
  }

  async authenticate(request) {
    const remoteAddress = request.socket.remoteAddress;
    const isLoopback = remoteAddress === "127.0.0.1" || remoteAddress === "::1" || remoteAddress === "::ffff:127.0.0.1";
    if (this.devBypass && isLoopback) {
      const email = String(request.headers["x-hd2-dev-user"] || "developer@localhost").slice(0, 200);
      return { email, subject: "development" };
    }
    return this.verifyToken(request.headers["cf-access-jwt-assertion"]);
  }
}
