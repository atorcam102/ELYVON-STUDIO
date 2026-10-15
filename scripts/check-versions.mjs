// Verifies that every manifest declares the same application version and,
// when a tag is given (`node scripts/check-versions.mjs v0.1.0`), that it matches.
import { readFileSync } from "node:fs";

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");

const pkg = JSON.parse(read("package.json")).version;
const tauri = JSON.parse(read("src-tauri/tauri.conf.json")).version;
const cargoMatch = read("Cargo.toml").match(/\[workspace\.package\][^[]*?\nversion\s*=\s*"([^"]+)"/s);
const cargo = cargoMatch?.[1];

const versions = { "package.json": pkg, "src-tauri/tauri.conf.json": tauri, "Cargo.toml (workspace)": cargo };
const unique = new Set(Object.values(versions));
let ok = unique.size === 1 && !unique.has(undefined);

const tag = process.argv[2];
if (tag !== undefined && tag.replace(/^v/, "") !== pkg) {
  console.error(`Tag ${tag} does not match application version ${pkg}.`);
  ok = false;
}
if (!ok) {
  console.error("Version mismatch:", versions);
  process.exit(1);
}
console.log(`All manifests at version ${pkg}${tag ? ` (tag ${tag})` : ""}.`);
