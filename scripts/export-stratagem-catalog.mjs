import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const indexSource = fs.readFileSync(path.join(root, "ui", "index.html"), "utf8");
const databaseStart = indexSource.indexOf("const defaultStratagemDB = [");
const databaseEnd = indexSource.indexOf("let stratagemDB", databaseStart);
if (databaseStart < 0 || databaseEnd < 0) throw new Error("Could not locate defaultStratagemDB");

const context = vm.createContext({});
vm.runInContext(
  `${indexSource.slice(databaseStart, databaseEnd)}\nglobalThis.database = defaultStratagemDB;`,
  context,
  { filename: "index.html:stratagem-database" },
);
const database = JSON.parse(vm.runInContext("JSON.stringify(database)", context));
const cleanTerms = (value) => Array.isArray(value)
  ? value.filter((entry) => typeof entry === "string" && entry.trim()).map((entry) => entry.trim())
  : [];
const seed = {
  schemaVersion: 1,
  catalogVersion: 0,
  items: database.map((item, order) => ({
    id: item.id,
    grp: item.grp,
    name: item.name,
    aliases: cleanTerms(item.aliases),
    ocr: cleanTerms(item.ocr),
    seq: item.seq,
    icon: { kind: "bundled", value: item.icon },
    enabled: true,
    order,
  })),
};

const output = path.join(root, "server", "stratagem-admin", "data", "seed-catalog.json");
fs.mkdirSync(path.dirname(output), { recursive: true });
fs.writeFileSync(output, `${JSON.stringify(seed, null, 2)}\n`, "utf8");

const bundledIconPattern = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}\.(?:svg|png)$/i;
const bundledIcons = [...new Set(seed.items.map((item) => item.icon.value))].sort();
const bundledIconRoot = path.join(root, "server", "stratagem-admin", "bundled-icons");
const temporaryIconRoot = `${bundledIconRoot}.tmp`;
fs.rmSync(temporaryIconRoot, { recursive: true, force: true });
fs.mkdirSync(temporaryIconRoot, { recursive: true });
for (const filename of bundledIcons) {
  if (!bundledIconPattern.test(filename)) throw new Error(`Unsafe bundled icon filename: ${filename}`);
  const source = path.join(root, "ui", filename);
  if (!fs.statSync(source).isFile()) throw new Error(`Bundled icon is missing: ${source}`);
  fs.copyFileSync(source, path.join(temporaryIconRoot, filename));
}
fs.rmSync(bundledIconRoot, { recursive: true, force: true });
fs.renameSync(temporaryIconRoot, bundledIconRoot);

process.stdout.write(
  `Exported ${seed.items.length} stratagems and ${bundledIcons.length} bundled icons to ${path.relative(root, output)}\n`,
);
