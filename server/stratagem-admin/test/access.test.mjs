import assert from "node:assert/strict";
import { generateKeyPairSync, sign } from "node:crypto";
import test from "node:test";
import { AccessVerifier } from "../lib/access.mjs";

const { privateKey, publicKey } = generateKeyPairSync("rsa", { modulusLength: 2048 });
const jwk = publicKey.export({ format: "jwk" });
jwk.kid = "test-key";
jwk.alg = "RS256";
jwk.use = "sig";

function token(overrides = {}) {
  const now = Math.floor(Date.now() / 1000);
  const header = Buffer.from(JSON.stringify({ alg: "RS256", kid: jwk.kid, typ: "JWT" })).toString("base64url");
  const payload = Buffer.from(JSON.stringify({
    iss: "https://example.cloudflareaccess.com",
    aud: ["expected-audience"],
    email: "owner@example.com",
    sub: "subject",
    nbf: now - 10,
    exp: now + 300,
    ...overrides,
  })).toString("base64url");
  const signature = sign("RSA-SHA256", Buffer.from(`${header}.${payload}`), privateKey).toString("base64url");
  return `${header}.${payload}.${signature}`;
}

function verifier() {
  return new AccessVerifier({
    teamDomain: "https://example.cloudflareaccess.com",
    audience: "expected-audience",
    allowedEmails: ["owner@example.com"],
    jwksProvider: async () => [jwk],
  });
}

test("valid Cloudflare Access JWT is accepted", async () => {
  assert.deepEqual(await verifier().verifyToken(token()), { email: "owner@example.com", subject: "subject" });
});

test("production configuration requires exactly one allowed administrator", () => {
  assert.equal(verifier().configured(), true);
  assert.equal(new AccessVerifier({
    teamDomain: "https://example.cloudflareaccess.com",
    audience: "expected-audience",
  }).configured(), false);
  assert.equal(new AccessVerifier({
    teamDomain: "https://example.cloudflareaccess.com",
    audience: "expected-audience",
    allowedEmails: ["one@example.com", "two@example.com"],
  }).configured(), false);
});

test("wrong audience, issuer, email and expiry are rejected", async () => {
  await assert.rejects(verifier().verifyToken(token({ aud: "wrong" })), /audience/);
  await assert.rejects(verifier().verifyToken(token({ iss: "https://wrong.cloudflareaccess.com" })), /issuer/);
  await assert.rejects(verifier().verifyToken(token({ email: "other@example.com" })), /not allowed/);
  await assert.rejects(verifier().verifyToken(token({ exp: 1 })), /expired/);
});

test("development bypass works only for loopback requests", async () => {
  const access = new AccessVerifier({ devBypass: true });
  const identity = await access.authenticate({ socket: { remoteAddress: "127.0.0.1" }, headers: {} });
  assert.equal(identity.subject, "development");
  await assert.rejects(access.authenticate({ socket: { remoteAddress: "10.0.0.2" }, headers: {} }));
});
