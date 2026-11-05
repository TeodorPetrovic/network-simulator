// GNS3 3.1 controller routes: appliances, blank QEMU disk, prune, template base configs.
import test from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { startServer, templateByType } from "./harness.mjs";

test("base configs, qemu disk creation, appliances and prune", async (t) => {
  // Throwaway data/image dirs: prune deletes files, so never point this at real images.
  const data = mkdtempSync(join(tmpdir(), "ns-compat-"));
  t.after(() => rmSync(data, { recursive: true, force: true }));
  const server = await startServer({
    env: { NETWORK_SIMULATOR_DATA_HOME: data, NETWORK_SIMULATOR_IMAGE_HOME: join(data, "images") },
  });
  t.after(() => server.stop());
  const { api } = server;

  // base configs hang off a template
  const tpl = await templateByType(server, "vpcs");
  const id = tpl.template_id;
  assert.deepEqual((await api("GET", `/templates/${id}/base-configs`)).json, []);
  const put = await api("PUT", `/templates/${id}/base-config/startup.vpc`, { content: "ip dhcp\n" });
  assert.equal(put.status, 200, JSON.stringify(put.json));
  assert.deepEqual((await api("GET", `/templates/${id}/base-configs`)).json, ["startup.vpc"]);
  assert.equal((await api("GET", `/templates/${id}/base-config/startup.vpc`)).json.content, "ip dhcp\n");
  assert.equal((await api("GET", `/templates/${id}/base-config/missing.vpc`)).status, 404);
  assert.equal((await api("PUT", `/templates/${id}/base-config/.hidden`, { content: "x" })).status, 422);
  assert.equal((await api("PUT", `/templates/${id}/base-config/a.cfg`, {})).status, 422);
  assert.equal((await api("GET", `/templates/00000000-0000-4000-8000-000000000000/base-configs`)).status, 404);

  // blank qemu disk
  const name = `e2e-blank-${Date.now()}.qcow2`;
  const created = await api("POST", `/images/qemu/${name}`, { format: "qcow2", size: 8 });
  assert.equal(created.status, 201, JSON.stringify(created.json));
  assert.ok(existsSync(created.json.path));
  assert.equal((await api("POST", `/images/qemu/${name}`, { format: "qcow2", size: 8 })).status, 422, "no overwrite");
  assert.equal((await api("POST", `/images/qemu/raw.img`, { format: "raw", size: 8 })).status, 422);
  assert.equal((await api("POST", `/images/qemu/..%2Fescape.qcow2`, { format: "qcow2", size: 8 })).status, 422);

  // an unreferenced image is pruned
  assert.equal((await api("DELETE", "/images/prune")).status, 204);
  assert.ok(!existsSync(created.json.path), "unreferenced image removed");

  // appliances
  const list = await api("GET", "/appliances");
  assert.equal(list.status, 200);
  assert.ok(Array.isArray(list.json));
  assert.equal((await api("GET", "/appliances/does-not-exist")).status, 404);
  assert.equal((await api("POST", "/appliances/does-not-exist/install")).status, 404);
});
