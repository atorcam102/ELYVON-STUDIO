// Minimal W3C WebDriver client for driving the real Elyvon Studio binary via
// `tauri-driver` (Linux: WebKitWebDriver; Windows: msedgedriver). No
// third-party dependencies, so the end-to-end suite stays light.

import { spawn } from "node:child_process";
import { once } from "node:events";
import { setTimeout as sleep } from "node:timers/promises";

const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";

export class Driver {
  constructor(base, sessionId) {
    this.base = base;
    this.sessionId = sessionId;
  }

  static async start({ application, port = 4444 }) {
    const proc = spawn("tauri-driver", ["--port", String(port)], { stdio: ["ignore", "pipe", "pipe"] });
    proc.stderr.on("data", (d) => process.env.E2E_DEBUG && process.stderr.write(d));
    const base = `http://127.0.0.1:${port}`;
    for (let i = 0; i < 50; i++) {
      try {
        await fetch(`${base}/status`);
        break;
      } catch {
        await sleep(100);
      }
    }
    const res = await request(base, "POST", "/session", {
      capabilities: { alwaysMatch: { "tauri:options": { application } } },
    });
    const driver = new Driver(base, res.sessionId);
    driver.proc = proc;
    return driver;
  }

  cmd(method, path, body) {
    return request(this.base, method, `/session/${this.sessionId}${path}`, body);
  }

  async stop() {
    try {
      await request(this.base, "DELETE", `/session/${this.sessionId}`);
    } finally {
      if (this.proc && this.proc.exitCode === null && this.proc.signalCode === null) {
        const exited = once(this.proc, "exit", { signal: AbortSignal.timeout(5000) });
        this.proc.kill();
        await exited;
      }
    }
  }

  async find(css, { timeout = 5000 } = {}) {
    const deadline = Date.now() + timeout;
    for (;;) {
      try {
        const v = await this.cmd("POST", "/element", { using: "css selector", value: css });
        return v[ELEMENT];
      } catch (e) {
        if (Date.now() > deadline) throw new Error(`element not found: ${css} (${e.message})`);
        await sleep(100);
      }
    }
  }

  async findAll(css) {
    const v = await this.cmd("POST", "/elements", { using: "css selector", value: css });
    return v.map((e) => e[ELEMENT]);
  }

  async count(css) {
    return (await this.findAll(css)).length;
  }

  click(id) {
    return this.cmd("POST", `/element/${id}/click`, {});
  }

  async clickCss(css) {
    return this.click(await this.find(css));
  }

  text(id) {
    return this.cmd("GET", `/element/${id}/text`);
  }

  rect(id) {
    return this.cmd("GET", `/element/${id}/rect`);
  }

  attr(id, name) {
    return this.cmd("GET", `/element/${id}/attribute/${name}`);
  }

  /** Runs JavaScript in the page. `args` are passed as `arguments`. */
  exec(script, args = []) {
    return this.cmd("POST", "/execute/sync", { script, args });
  }

  /** Runs an async function body; resolve via the last argument. */
  execAsync(script, args = []) {
    return this.cmd("POST", "/execute/async", { script, args });
  }

  /** Invokes a native command exactly as the UI does. */
  invoke(command, payload = {}) {
    return this.execAsync(
      `const done = arguments[arguments.length - 1];
       window.__TAURI_INTERNALS__.invoke(arguments[0], arguments[1]).then(
         (v) => done({ ok: v === undefined ? null : v }),
         (e) => done({ err: e }));`,
      [command, payload],
    );
  }

  /** Types into an element (W3C "Element Send Keys"). */
  type(id, text) {
    return this.cmd("POST", `/element/${id}/value`, { text });
  }

  /**
   * Rendered text of the first match once non-empty. Uses `innerText` because
   * WebKitWebDriver's "Get Element Text" returns "" for some styled spans.
   */
  textOf(css, opts) {
    return this.waitFor(
      async () => (await this.exec("return document.querySelector(arguments[0])?.innerText ?? ''", [css])) || null,
      { message: `text of ${css}`, ...opts },
    );
  }

  keys(text) {
    return this.cmd("POST", "/actions", {
      actions: [
        {
          type: "key",
          id: "keyboard",
          actions: [...text].flatMap((c) => [
            { type: "keyDown", value: c },
            { type: "keyUp", value: c },
          ]),
        },
      ],
    });
  }

  /** Pointer drag from the centre of `id` by (dx, dy) CSS pixels. */
  async drag(id, dx, dy, steps = 8) {
    const r = await this.rect(id);
    const x = Math.round(r.x + Math.min(20, r.width / 2));
    const y = Math.round(r.y + r.height / 2);
    const moves = [];
    for (let i = 1; i <= steps; i++) {
      moves.push({ type: "pointerMove", duration: 16, origin: "viewport", x: Math.round(x + (dx * i) / steps), y: Math.round(y + (dy * i) / steps) });
    }
    await this.cmd("POST", "/actions", {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", duration: 0, origin: "viewport", x, y },
            { type: "pointerDown", button: 0 },
            ...moves,
            { type: "pointerUp", button: 0 },
          ],
        },
      ],
    });
    await this.cmd("DELETE", "/actions");
  }

  async doubleClick(id) {
    const r = await this.rect(id);
    const x = Math.round(r.x + Math.min(20, r.width / 2));
    const y = Math.round(r.y + r.height / 2);
    await this.cmd("POST", "/actions", {
      actions: [
        {
          type: "pointer",
          id: "mouse",
          parameters: { pointerType: "mouse" },
          actions: [
            { type: "pointerMove", duration: 0, origin: "viewport", x, y },
            { type: "pointerDown", button: 0 },
            { type: "pointerUp", button: 0 },
            { type: "pointerDown", button: 0 },
            { type: "pointerUp", button: 0 },
          ],
        },
      ],
    });
    await this.cmd("DELETE", "/actions");
  }

  async screenshot() {
    return Buffer.from(await this.cmd("GET", "/screenshot"), "base64");
  }

  async waitFor(predicate, { timeout = 5000, message = "condition" } = {}) {
    const deadline = Date.now() + timeout;
    for (;;) {
      const v = await predicate();
      if (v) return v;
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${message}`);
      await sleep(100);
    }
  }
}

async function request(base, method, path, body) {
  const res = await fetch(base + path, {
    method,
    headers: body ? { "content-type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const json = await res.json().catch(() => ({}));
  if (!res.ok || json.value?.error) {
    throw new Error(`${method} ${path}: ${json.value?.error ?? res.status} ${json.value?.message ?? ""}`);
  }
  return json.value;
}
