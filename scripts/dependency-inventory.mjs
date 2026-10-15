// Reproducible inventory; licence identifiers are metadata, not legal clearance.
import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";

const allowedNpm = new Set([
  "MIT", "Apache-2.0", "Apache-2.0 OR MIT", "MIT OR Apache-2.0", "ISC",
  "CC-BY-4.0", "BSD-2-Clause", "BSD-3-Clause", "MPL-2.0", "BlueOak-1.0.0",
]);
const lock = JSON.parse(readFileSync("package-lock.json", "utf8"));
const npm = Object.entries(lock.packages).filter(([path]) => path).map(([path, pkg]) => ({
  path, name: path.split("node_modules/").at(-1), version: pkg.version, license: pkg.license ?? null,
  integrity: pkg.integrity ?? null, resolved: pkg.resolved ?? null, dev: !!pkg.dev, optional: !!pkg.optional,
}));
for (const pkg of npm) {
  if (!allowedNpm.has(pkg.license)) throw new Error(`Review npm licence: ${pkg.path}: ${pkg.license}`);
  if (!pkg.integrity || !pkg.resolved?.startsWith("https://registry.npmjs.org/")) throw new Error(`Review npm source/integrity: ${pkg.path}`);
}
const metadata = JSON.parse(execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], { encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }));
const rust = metadata.packages.map((pkg) => ({
  name: pkg.name, version: pkg.version, license: pkg.license, source: pkg.source,
  workspace: metadata.workspace_members.includes(pkg.id),
})).sort((a, b) => `${a.name}@${a.version}`.localeCompare(`${b.name}@${b.version}`));
const report = { schemaVersion: 1, npm, rust, limitations: [
  "All lockfile entries, including development and optional/platform dependencies.",
  "Rust licence/source/advisory policy is enforced separately by cargo deny.",
  "Not a list of dynamically linked OS libraries or a substitute for full distribution notices.",
] };
const output = process.argv[2] ?? "dependency-inventory.json";
writeFileSync(output, JSON.stringify(report, null, 2) + "\n");
console.log(`Inventory: ${npm.length} npm lock entries, ${rust.length} Rust packages -> ${output}`);
