#!/usr/bin/env node
// Screenshots for the design review (web/DESIGN.md, Checks). Drives headless Chrome over the
// DevTools protocol (Node 22's built-in WebSocket, no dependencies) so that each shot waits for
// the page's own data instead of a fixed timer, and the viewport is emulated at the exact width.
//
//   home      the flood drill on its last state, and the reduced-motion table
//   book      the lender book
//   station   one station
// at 375, 820 and 1280 px, once with the live exchange price and once with a test price that
// puts the water over flood stage. Needs the local stack: pnpm local:chain, local:deploy, policy, web.
import { spawn } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const WEB = process.env.WEB_URL ?? "http://localhost:5174";
const API = process.env.POLICY_URL ?? "http://127.0.0.1:8787";
const CHROME = process.env.CHROME ?? "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome";
const FLOOD_PRICE = Number(process.env.FLOOD_PRICE ?? 650);
const OUT = join(root, "web", "screens");
const WIDTHS = [375, 820, 1280];

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function ensureUp() {
  if (!existsSync(CHROME)) throw new Error(`Chrome not found at ${CHROME}; set CHROME`);
  for (const [name, url] of [["web app", WEB], ["policy signer", `${API}/healthz`]]) {
    const ok = await fetch(url).then((r) => r.ok).catch(() => false);
    if (!ok) throw new Error(`${name} is not running at ${url}`);
  }
}

async function setPrice(usd) {
  await fetch(`${API}/v1/dev/price`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ usd }),
  });
}

async function launch() {
  const profile = mkdtempSync(join(tmpdir(), "lentic-screens-"));
  const chrome = spawn(CHROME, [
    "--headless=new",
    "--disable-gpu",
    "--hide-scrollbars",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "about:blank",
  ]);
  let port;
  for (let i = 0; i < 100 && !port; i++) {
    await sleep(100);
    const f = join(profile, "DevToolsActivePort");
    if (existsSync(f)) port = readFileSync(f, "utf8").split("\n")[0];
  }
  if (!port) throw new Error("Chrome did not open a DevTools port");
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  const page = targets.find((t) => t.type === "page");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  let id = 0;
  const pending = new Map();
  ws.onmessage = (m) => {
    const msg = JSON.parse(m.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      msg.error ? reject(new Error(msg.error.message)) : resolve(msg.result);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const n = ++id;
      pending.set(n, { resolve, reject });
      ws.send(JSON.stringify({ id: n, method, params }));
    });
  const close = async () => {
    ws.close();
    const exited = new Promise((resolve) => chrome.once("exit", resolve));
    chrome.kill();
    await exited;
    // Chrome may still be flushing its profile for a moment after exit.
    for (let i = 0; i < 10; i++) {
      try {
        rmSync(profile, { recursive: true, force: true });
        break;
      } catch {
        await sleep(200);
      }
    }
  };
  return { send, close };
}

async function shot(cdp, { name, url, width, waitFor, reduced = false }) {
  await cdp.send("Emulation.setDeviceMetricsOverride", { width, height: 900, deviceScaleFactor: 1, mobile: width < 500 });
  await cdp.send("Emulation.setEmulatedMedia", {
    features: [{ name: "prefers-reduced-motion", value: reduced ? "reduce" : "no-preference" }],
  });
  await cdp.send("Page.navigate", { url: "about:blank" });
  await cdp.send("Page.navigate", { url });
  const deadline = Date.now() + 20_000;
  for (;;) {
    const { result } = await cdp.send("Runtime.evaluate", {
      expression: `!!document.querySelector(${JSON.stringify(waitFor)})`,
      returnByValue: true,
    });
    if (result.value) break;
    if (Date.now() > deadline) throw new Error(`${name}: ${waitFor} never appeared`);
    await sleep(200);
  }
  await sleep(700); // let the water line settle (220 ms) and fonts paint
  const { result } = await cdp.send("Runtime.evaluate", {
    expression: "document.documentElement.scrollHeight",
    returnByValue: true,
  });
  await cdp.send("Emulation.setDeviceMetricsOverride", {
    width,
    height: Math.ceil(result.value),
    deviceScaleFactor: 1,
    mobile: width < 500,
  });
  await sleep(150);
  const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(OUT, `${name}.png`), Buffer.from(data, "base64"));
  console.log(`  web/screens/${name}.png`);
}

async function main() {
  await ensureUp();
  mkdirSync(OUT, { recursive: true });
  const loanId = JSON.parse(readFileSync(join(root, "dev", "loan.local.json"), "utf8")).loanId;
  const { roundSecs } = await (await fetch(`${API}/v1/info`)).json();
  const cdp = await launch();
  try {
    await cdp.send("Page.enable");
    for (const state of ["live", "flood"]) {
      console.log(state === "live" ? "live price" : `test price ${FLOOD_PRICE} USD (flood stage)`);
      await setPrice(state === "live" ? null : FLOOD_PRICE);
      await sleep((roundSecs + 2) * 1000);
      for (const width of WIDTHS) {
        if (state === "live") {
          await shot(cdp, { name: `home-${width}-drill`, url: `${WEB}/#/?drill=end`, width, waitFor: ".drill-split.shown" });
          await shot(cdp, { name: `home-${width}-reduced`, url: `${WEB}/#/`, width, waitFor: ".drill-static", reduced: true });
        }
        await shot(cdp, { name: `book-${width}-${state}`, url: `${WEB}/#/book`, width, waitFor: ".stations tbody tr" });
        await shot(cdp, {
          name: `station-${width}-${state}`,
          url: `${WEB}/#/station/${loanId}`,
          width,
          waitFor: ".readings-list",
        });
      }
    }
  } finally {
    await setPrice(null).catch(() => undefined);
    await cdp.close();
  }
  console.log("done; the policy signer is back on the exchange price");
}

main().catch((e) => {
  console.error(e.message);
  process.exit(1);
});
