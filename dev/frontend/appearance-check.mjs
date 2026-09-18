// Repository-owned Broadway + headless viewer; never drives the real desktop.
// Usage: node dev/frontend/appearance-check.mjs target/debug/deps/mirelay-HASH [test-filter]
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, writeFile, realpath } from "node:fs/promises";
import { resolve } from "node:path";
import { createServer } from "node:net";
import { once } from "node:events";

const root = await realpath(new URL("../../", import.meta.url).pathname);
const binary = await realpath(resolve(process.argv[2] ?? ""));
if (!binary.startsWith(`${root}/target/debug/deps/mirelay-`)) throw Error("Use the repository test binary");
// Headless browser focus is not reliable enough for motion lifecycle tests.
// Use appearance-x11-check.py for those; Broadway checks layout/resources only.
const filter = process.argv[3] ?? "desktop::ui_v2_tests::native_v2_preserves_details_and_category_switches";
if (!["desktop::ui_v2_tests::native_v2_preserves_details_and_category_switches",
      "desktop::ui_v2_tests::native_connection_states_gate_receive_and_update_empty_copy"].includes(filter)) throw Error("Use a supported isolated native UI fixture test");
await mkdir(`${root}/target/ui-v2-appearance`, { recursive: true });
const report = await mkdtemp(`${root}/target/ui-v2-appearance/run-`);
console.log(`REPORT=${report}`);
const freePort = async () => { const server = createServer(); server.listen(0, "127.0.0.1"); await once(server, "listening"); const port = server.address().port; await new Promise(r => server.close(r)); return port; };
const port = await freePort();
const children = [];
let socket;
const pause = ms => new Promise(r => setTimeout(r, ms));
function child(command, args, env = process.env) {
  const process = spawn(command, args, { env, stdio: ["ignore", "pipe", "pipe"] });
  children.push(process); return process;
}
try {
  const broadway = child("gtk4-broadwayd", [":96", "-p", String(port), "-a", "127.0.0.1"]);
  let broadwayLog = ""; for (const stream of [broadway.stdout, broadway.stderr]) stream.on("data", b => broadwayLog += b);
  for (let i = 0; ; i++) {
    if (broadway.exitCode !== null || i > 80) throw Error(`Broadway failed: ${broadwayLog}`);
    if (await fetch(`http://127.0.0.1:${port}`).then(r => r.ok).catch(() => false)) break;
    await pause(100);
  }
  const chrome = child("google-chrome", ["--headless=new", "--disable-gpu", "--disable-background-networking", "--disable-dev-shm-usage", "--no-first-run", "--no-proxy-server", "--remote-debugging-port=0", `--user-data-dir=${report}/chrome-profile`, "about:blank"]);
  let chromeLog = ""; chrome.stderr.on("data", b => chromeLog += b);
  let endpoint;
  for (let i = 0; !(endpoint = chromeLog.match(/DevTools listening on (ws:\/\/\S+)/)?.[1]); i++) {
    if (chrome.exitCode !== null || i > 100) throw Error(`Chrome failed: ${chromeLog}`);
    await pause(100);
  }
  socket = new WebSocket(endpoint); await once(socket, "open");
  let sequence = 0; const requests = new Map(); const browserErrors = [];
  socket.addEventListener("message", event => {
    const message = JSON.parse(event.data);
    if (message.id) { const handler = requests.get(message.id); requests.delete(message.id); if (message.error) handler?.reject(Error(JSON.stringify(message.error))); else handler?.resolve(message.result); }
    if (message.method === "Runtime.exceptionThrown") browserErrors.push(message.params);
  });
  async function send(method, params = {}, sessionId) {
    const id = ++sequence;
    const pending = new Promise((resolve, reject) => requests.set(id, { resolve, reject }));
    socket.send(JSON.stringify({ id, method, params, sessionId }));
    return await pending;
  }
  const { targetId } = await send("Target.createTarget", { url: `http://127.0.0.1:${port}` });
  const { sessionId } = await send("Target.attachToTarget", { targetId, flatten: true });
  await send("Runtime.enable", {}, sessionId);
  await send("Page.enable", {}, sessionId);
  await send("Page.bringToFront", {}, sessionId);
  await send("Emulation.setFocusEmulationEnabled", { enabled: true }, sessionId);
  await send("Emulation.setDeviceMetricsOverride", { width: 1200, height: 850, deviceScaleFactor: 1, mobile: false }, sessionId);
  await pause(700);
  const env = { ...process.env, GDK_BACKEND: "broadway", BROADWAY_DISPLAY: ":96", GTK_A11Y: "none", G_DEBUG: "fatal-criticals", MIRELAY_APPEARANCE_QA_DIR: report };
  delete env.GSK_RENDERER;
  const test = child(binary, [filter, "--ignored", "--exact", "--nocapture", "--test-threads=1"], env);
  let output = ""; for (const stream of [test.stdout, test.stderr]) stream.on("data", b => { output += b; process.stdout.write(b); });
  const ended = once(test, "exit");
  await pause(1000);
  // Focus only this disposable browser window, before the native motion check.
  // The native fixtures do not attach transfer handlers to this button. A
  // focusable control gives GTK keyboard focus; blank canvas clicks need not.
  await send("Input.dispatchMouseEvent", { type: "mouseMoved", x: 740, y: 62 }, sessionId);
  for (const type of ["mousePressed", "mouseReleased"]) await send("Input.dispatchMouseEvent", { type, x: 740, y: 62, button: "left", clickCount: 1 }, sessionId);
  const screenshot = await send("Page.captureScreenshot", {}, sessionId);
  await writeFile(`${report}/viewer.png`, Buffer.from(screenshot.data, "base64"));
  const [code] = await ended;
  await writeFile(`${report}/native-test.log`, output);
  await writeFile(`${report}/broadway.log`, broadwayLog);
  await writeFile(`${report}/browser-errors.json`, JSON.stringify(browserErrors, null, 2));
  if (code !== 0 || browserErrors.length) throw Error(`Native test exit ${code}; browser exceptions ${browserErrors.length}`);
  console.log("Native GTK check and browser exception check passed.");
} finally {
  socket?.close();
  for (const process of children.reverse()) if (process.exitCode === null) process.kill("SIGTERM");
}
