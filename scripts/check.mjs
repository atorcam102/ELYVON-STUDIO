// Same ordered checks locally and in CI. Never hide a failing exit code.
import { spawnSync } from "node:child_process";

const mode = process.argv[2] ?? "frontend";
const frontend = [
  ["node", ["scripts/check-versions.mjs"]],
  ...["typecheck", "lint", "test", "build"].map((name) => ["npm", ["run", name]]),
];
const rust = [
  ["cargo", ["fmt", "--all", "--check"]],
  ["cargo", ["clippy", "--workspace", "--all-targets", "--locked", "--", "-D", "warnings"]],
  ["cargo", ["test", "--workspace", "--locked"]],
  ["git", ["add", "-N", "src/bindings"]],
  ["git", ["diff", "--exit-code", "--", "src/bindings"]],
];
if (!["frontend", "rust"].includes(mode)) throw new Error(`unknown check mode: ${mode}`);
for (const [command, args] of mode === "rust" ? rust : frontend) {
  console.log(`> ${command} ${args.join(" ")}`);
  const result = spawnSync(process.platform === "win32" && command === "npm" ? "npm.cmd" : command, args, { stdio: "inherit", shell: process.platform === "win32" && command === "npm" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}
