// Real native binary only. Linux /proc + WebKit WebDriver, no IPC mocks.
// Run AFTER building: xvfb-run -a dbus-run-session -- node e2e/measure.mjs
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, readlinkSync, writeFileSync } from "node:fs";
import { tmpdir, platform, arch, cpus, totalmem } from "node:os";
import { basename, join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { setTimeout as sleep } from "node:timers/promises";
import { Driver } from "./webdriver.mjs";

const app = resolve(process.env.ELYVON_APP ?? "target/debug/elyvon-studio");
const output = resolve(process.env.E2E_METRICS ?? "e2e-evidence/native-metrics.json");
const home = mkdtempSync(join(tmpdir(), "elyvon-measure-"));
for (const name of ["CONFIG", "DATA", "CACHE"]) process.env[`XDG_${name}_HOME`] = join(home, name.toLowerCase());

function descendants(pid) {
  let children = [];
  try { children = readFileSync(`/proc/${pid}/task/${pid}/children`, "utf8").trim().split(/\s+/).filter(Boolean).map(Number); } catch { /* exited */ }
  return children.flatMap((child) => [child, ...descendants(child)]);
}
function memory(driver) {
  const root = descendants(driver.proc.pid).find((pid) => {
    try { return basename(readlinkSync(`/proc/${pid}/exe`)) === basename(app); } catch { return false; }
  });
  assert.ok(root, "native app PID must be observable, not guessed");
  const processes = [root, ...descendants(root)].flatMap((pid) => {
    try {
      const status = readFileSync(`/proc/${pid}/status`, "utf8");
      return [{ pid, name: status.match(/^Name:\s+(.+)$/m)?.[1], rssKiB: Number(status.match(/^VmRSS:\s+(\d+) kB$/m)?.[1]) }];
    } catch { return []; }
  });
  assert.ok(processes.length && processes.every((p) => Number.isFinite(p.rssKiB)), "RSS must come from /proc");
  return { processes, aggregateRssKiB: processes.reduce((sum, p) => sum + p.rssKiB, 0) };
}

const report = {
  schemaVersion: 1,
  timestamp: new Date().toISOString(), commit: process.env.GITHUB_SHA ?? null,
  environment: { platform: platform(), arch: arch(), cpu: cpus()[0]?.model, logicalCpus: cpus().length, totalMemoryBytes: totalmem(), binary: app, build: "debug", display: "Xvfb (virtual display)" },
  limitations: ["Startup includes tauri-driver startup, WebDriver session creation and IPC readiness; not executable-only launch time.", "RSS sums native app and observed WebKit descendants; shared pages can be counted multiple times. One-second idle, meters off.", "Frame intervals under Xvfb are diagnostic, not hardware FPS, audio latency or a release-build performance guarantee.", "Three sequential warm-cache launches; no cold-cache claim."],
  launches: [],
};
let driver;
try {
  for (let run = 0; run < 3; run++) {
    const start = performance.now();
    driver = await Driver.start({ application: app });
    await driver.find(".controlbar", { timeout: 20_000 });
    const session = await driver.invoke("project_get");
    assert.ok(session.ok, JSON.stringify(session.err));
    const launchToReadyMs = performance.now() - start;
    await sleep(1000);
    report.launches.push({ launchToReadyMs, idleMemory: memory(driver) });
    if (run < 2) { await driver.stop(); driver = null; }
  }
  const opened = await driver.invoke("project_open_demo", { discardChanges: true });
  assert.ok(opened.ok, JSON.stringify(opened.err));
  await driver.exec("location.reload()");
  await driver.find(".region", { timeout: 10_000 });
  report.timeline = await driver.execAsync(`
    const done = arguments[arguments.length - 1];
    const scroller = document.querySelector('.timeline__scroll');
    if (!scroller) return done({ error: 'missing timeline' });
    const original = scroller.scrollLeft, intervals = [], total = 120;
    const max = scroller.scrollWidth - scroller.clientWidth;
    let previous = null;
    function frame(time) {
      if (previous !== null) intervals.push(time - previous);
      previous = time;
      scroller.scrollLeft = (intervals.length / total) * max;
      if (intervals.length < total) return requestAnimationFrame(frame);
      scroller.scrollLeft = original;
      const sorted = [...intervals].sort((a, b) => a - b);
      done({ frameIntervalsMs: intervals, medianMs: sorted[Math.floor(sorted.length / 2)], p95Ms: sorted[Math.ceil(sorted.length * .95) - 1], maxScrollLeft: max, viewport: { width: innerWidth, height: innerHeight } });
    }
    requestAnimationFrame(frame);
  `);
  assert.equal(report.timeline.frameIntervalsMs?.length, 120, JSON.stringify(report.timeline));
  assert.ok(report.timeline.maxScrollLeft > 0, "benchmark must actually scroll");
  report.timeline.dataset = { name: "Rust demo_project", tracks: opened.ok.project.tracks.length, regions: opened.ok.project.tracks.reduce((n, t) => n + t.regions.length, 0), lengthBars: opened.ok.project.lengthBars };
  report.status = "measured";
} finally {
  await driver?.stop();
  mkdirSync(resolve(output, ".."), { recursive: true });
  writeFileSync(output, JSON.stringify(report, null, 2) + "\n");
}
console.log(JSON.stringify(report, null, 2));
if (process.env.GITHUB_ACTIONS) {
  // Read-only check annotation lets reviewers retrieve the same raw JSON via
  // GitHub's API when the artifact storage endpoint is unavailable.
  const data = JSON.stringify(report).replaceAll("%", "%25").replaceAll("\r", "%0D").replaceAll("\n", "%0A");
  console.log(`::notice title=Native metrics::${data}`);
}
// A small, directly retrievable CI summary (the full report remains an artifact).
if (process.env.GITHUB_STEP_SUMMARY) {
  const { appendFileSync } = await import("node:fs");
  appendFileSync(process.env.GITHUB_STEP_SUMMARY, `### Native measurements (debug, Linux x64, Xvfb)\n\n${report.launches.map((l, i) => `- Launch ${i + 1}: ${l.launchToReadyMs.toFixed(1)} ms incl. WebDriver; aggregate idle RSS ${(l.idleMemory.aggregateRssKiB / 1024).toFixed(1)} MiB`).join("\n")}\n- Demo timeline: median ${report.timeline.medianMs.toFixed(2)} ms, p95 ${report.timeline.p95Ms.toFixed(2)} ms (120 frame intervals).\n\nSee artifact native-evidence for raw samples and limitations.\n`);
}
