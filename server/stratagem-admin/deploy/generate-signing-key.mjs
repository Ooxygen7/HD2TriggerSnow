import { generateKeyPairSync } from "node:crypto";
import { chmod, writeFile } from "node:fs/promises";

const privatePath = process.argv[2] || "/tmp/hd2-catalog-signing-private.pem";
const publicPath = process.argv[3] || "/tmp/hd2-catalog-signing-public.pem";
const { privateKey, publicKey } = generateKeyPairSync("ed25519");
await writeFile(privatePath, privateKey.export({ type: "pkcs8", format: "pem" }), { mode: 0o600, flag: "wx" });
await writeFile(publicPath, publicKey.export({ type: "spki", format: "pem" }), { mode: 0o644, flag: "wx" });
await chmod(privatePath, 0o600);
await chmod(publicPath, 0o644);
process.stdout.write(publicKey.export({ format: "jwk" }).x);
