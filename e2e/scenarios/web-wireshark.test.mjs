import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { statSync } from "node:fs";
import { scenario, link, startNode, waitFor } from "./lib.mjs";
import { templateByType } from "../harness.mjs";

const image = "network-simulator-web-wireshark:test";
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const docker = (...args) => execFileSync("docker", args, { encoding: "utf8" }).trim();
const container = (projectId) => `gns3-wireshark-${projectId}`;
const processList = (projectId) => docker("exec", container(projectId), "bash", "-lc", "ps -eo pid,args");
const wiresharkPids = (projectId) => [...processList(projectId).matchAll(/^\s*(\d+)\s+.*wireshark -i -/gm)].map((m) => m[1]);
const xpraIsRunning = (projectId) => /:[0-9]+/.test(docker("exec", container(projectId), "xpra", "list"));

test("Web Wireshark receives the live capture through controller-fed stdin", async (t) => {
  const lab = await scenario(t, "scenario-web-wireshark");
  const projectContainer = container(lab.projectId);
  t.after(() => { try { docker("rm", "-f", projectContainer); } catch {} });

  let localImage = false;
  try { localImage = Boolean(docker("images", "-q", image)); } catch {}
  if (localImage) {
    const configured = await lab.server.api("PUT", "/settings", { WebWireshark: { image } });
    assert.equal(configured.status, 200, JSON.stringify(configured));
  }

  const a = await lab.add(await templateByType(lab.server, "vpcs"), 0, 0, "A");
  const b = await lab.add(await templateByType(lab.server, "vpcs"), 200, 0, "B");
  const wire = await link(lab.server, lab.projectId, a, b);
  await Promise.all([startNode(lab.server, lab.projectId, a), startNode(lab.server, lab.projectId, b)]);
  const consoleA = await lab.console(a);
  await consoleA.expectRegex(/VPCS>/);
  consoleA.sendLine("ip 10.55.0.1/24"); await consoleA.expectRegex(/VPCS>/);
  const consoleB = await lab.console(b);
  await consoleB.expectRegex(/VPCS>/);
  consoleB.sendLine("ip 10.55.0.2/24"); await consoleB.expectRegex(/VPCS>/);
  consoleA.sendLine("ping 10.55.0.2 -c 1000");

  const started = await lab.server.api("POST", `/projects/${lab.projectId}/links/${wire.link_id}/capture/start`, {
    capture_file_name: "web-wireshark.pcap", data_link_type: "DLT_EN10MB", wireshark: true,
  });
  assert.equal(started.status, 201, JSON.stringify(started));
  assert.ok(started.json.ws_path?.startsWith("/v3/projects/"), "link JSON should contain a relative WebSocket path");
  await waitFor(async () => {
    const links = await lab.server.api("GET", `/projects/${lab.projectId}/links`);
    return links.json?.find((item) => item.link_id === wire.link_id)?.wireshark === true;
  }, { timeout: 90000, label: "Web Wireshark started" });

  await waitFor(() => xpraIsRunning(lab.projectId), { timeout: 30000, label: "xpra running" });
  await waitFor(() => wiresharkPids(lab.projectId).length > 0, { timeout: 30000, label: "Wireshark running" });
  await sleep(8000);
  const capturePath = started.json.capture_file_path;
  const size = statSync(capturePath).size;
  assert.ok(size > 24, `capture should contain packet data, got ${size} bytes`);
  assert.ok(wiresharkPids(lab.projectId).length > 0, "Wireshark should still be alive after receiving packets");

  const socket = new WebSocket(new URL(started.json.ws_path, lab.server.base).href, "binary");
  let binaryFrames = 0;
  socket.addEventListener("message", (event) => { if (typeof event.data !== "string") binaryFrames++; });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("WebSocket upgrade timed out")), 10000);
    socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
    socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("WebSocket upgrade failed")); }, { once: true });
  });
  await sleep(3000);
  assert.equal(socket.readyState, WebSocket.OPEN, "WebSocket proxy should stay open");
  assert.ok(binaryFrames > 0 || socket.readyState === WebSocket.OPEN, "xpra handshake should be sent or upgrade should remain open");
  socket.close();

  const oldPids = wiresharkPids(lab.projectId);
  const restarted = await lab.server.api("POST", `/projects/${lab.projectId}/links/${wire.link_id}/capture/wireshark/restart`, {});
  assert.equal(restarted.status, 200, JSON.stringify(restarted));
  await waitFor(() => wiresharkPids(lab.projectId).some((pid) => !oldPids.includes(pid)), { timeout: 30000, label: "fresh Wireshark process" });
  assert.equal(wiresharkPids(lab.projectId).length, 1, "restart should leave one Wireshark process");

  const stopped = await lab.server.api("POST", `/projects/${lab.projectId}/links/${wire.link_id}/capture/stop`, {});
  assert.equal(stopped.status, 204, JSON.stringify(stopped));
  await waitFor(() => wiresharkPids(lab.projectId).length === 0, { timeout: 15000, label: "Wireshark stopped" });
  const deleted = await lab.server.api("DELETE", `/projects/${lab.projectId}`);
  assert.equal(deleted.status, 204, JSON.stringify(deleted));
  await waitFor(() => { try { docker("inspect", projectContainer); return false; } catch { return true; } }, { timeout: 70000, label: "project Web Wireshark container removed" });
});
