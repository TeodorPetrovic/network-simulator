#!/usr/bin/env bun
// Deploy or verify the bundled lab library.
//   bun scripts/labs.mjs list
//   bun scripts/labs.mjs deploy <id|all> [--server URL] [--user admin --password-file data/initial-admin-password.txt] [--start]
//   bun scripts/labs.mjs verify <id|all>      (isolated throw-away controller, never touches your data)
import { readdirSync, readFileSync, mkdtempSync, rmSync, existsSync } from "node:fs";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawn, execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { tmpdir } from "node:os";
import net from "node:net";
import { Lab, client, login, routerOs, sleep } from "../labs/lib.mjs";
import { mkdirSync, writeFileSync, readdirSync as readdir, linkSync, copyFileSync } from "node:fs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const command = args.shift();
const flag = (name, fallback) => { const i = args.indexOf(`--${name}`); if (i < 0) return fallback; const v = args[i + 1]; args.splice(i, 2); return v; };
const has = (name) => { const i = args.indexOf(`--${name}`); if (i < 0) return false; args.splice(i, 1); return true; };

async function loadLabs() {
  const files = readdirSync(join(root, "labs")).filter((f) => /^\d\d-.*\.mjs$/.test(f)).sort();
  return Promise.all(files.map((f) => import(pathToFileURL(join(root, "labs", f)).href)));
}
const pick = (labs, which) => which === "all" || !which ? labs : labs.filter((l) => l.id === which || l.id.startsWith(which));

function freePort() {
  return new Promise((ok, fail) => { const s = net.createServer(); s.listen(0, "127.0.0.1", () => { const { port } = s.address(); s.close(() => ok(port)); }); s.on("error", fail); });
}
async function isolatedServer({ imageHome } = {}) {
  const bin = process.env.E2E_BINARY ?? join(root, "target/debug/network-simulator");
  const dir = mkdtempSync(join(tmpdir(), "ns-labs-"));
  const port = await freePort();
  const child = spawn(bin, [], { cwd: dir, stdio: ["ignore", "pipe", "pipe"], env: {
    ...process.env, PATH: `${process.env.HOME}/.local/share/network-simulator/bin:${process.env.PATH}`, NETWORK_SIMULATOR_BIND: `127.0.0.1:${port}`, NETWORK_SIMULATOR_DB: join(dir, "t.sqlite"), NETWORK_SIMULATOR_DATA_HOME: join(dir, "data"), NETWORK_SIMULATOR_IMAGE_HOME: imageHome ?? process.env.NETWORK_SIMULATOR_IMAGE_HOME ?? join(process.env.HOME, ".local/share/network-simulator/images"),
  } });
  let log = ""; child.stdout.on("data", (d) => { log += d; }); child.stderr.on("data", (d) => { log += d; });
  const base = `http://127.0.0.1:${port}`;
  for (let i = 0; ; i++) {
    try { if ((await fetch(`${base}/healthz`)).ok) break; } catch {}
    if (i > 150) { child.kill(); throw new Error(`server did not start\n${log.slice(-1500)}`); }
    await new Promise((r) => setTimeout(r, 100));
  }
  return { base, cwd: dir, async stop() { child.kill("SIGTERM"); await new Promise((r) => setTimeout(r, 400)); child.kill("SIGKILL"); rmSync(dir, { recursive: true, force: true }); } };
}

function ensureImages(lab) {
  // Registry appliances (lab.appliances) are imported and installed through the app by lab.device().
  for (const build of lab.builds ?? []) {
    try { execFileSync("docker", ["image", "inspect", build.tag], { stdio: "ignore" }); }
    catch { console.log(`  building ${build.tag}…`); execFileSync("docker", ["build", "-q", "-t", build.tag, join(root, build.context)], { stdio: "inherit" }); }
  }
  for (const image of lab.needs ?? []) {
    try { execFileSync("docker", ["image", "inspect", image], { stdio: "ignore" }); }
    catch { console.log(`  pulling ${image}…`); execFileSync("docker", ["pull", "-q", image], { stdio: "inherit" }); }
  }
}

function linkOrCopy(from, to) {
  try { linkSync(from, to); } catch { copyFileSync(from, to); }
}

/** Temp image home holding the qemu images and the c3725 IOS image the labs may reference. */
function stagedImageHome() {
  const home = mkdtempSync(join(tmpdir(), "ns-labs-images-"));
  const real = join(process.env.HOME, ".local/share/network-simulator/images");
  for (const kind of ["qemu", "dynamips"]) {
    mkdirSync(join(home, kind), { recursive: true });
    for (const name of readdir(join(real, kind))) {
      if (kind === "dynamips" && !/c3725/.test(name)) continue;
      linkOrCopy(join(real, kind, name), join(home, kind, name));
    }
  }
  return home;
}

async function packageLab(def, file) {
  const imageHome = stagedImageHome();
  const server = await isolatedServer({ imageHome });
  const lab = new Lab(client(server.base), `Lab ${def.id.slice(0, 2)} – ${def.title}`);
  try {
    await lab.create();
    await def.build(lab);
    for (const key of def.bake ?? []) lab.bakeKeys = [...(lab.bakeKeys ?? []), key];
    if (def.bake?.length) {
      console.log("  baking router configuration into disk images…");
      await lab.startAll(def.bake);
      await def.configure(lab);
      for (const key of def.bake) {
        const router = await routerOs(lab, lab.node(key));
        await router.shutdown();
        router.close();
      }
      await sleep(3000);
      await lab.stopAll();
      await sleep(2000);
      await lab.refresh();
      for (const key of def.bake) {
        const node = lab.node(key);
        if (!node.runtime_overlay) throw new Error(`${key} has no runtime overlay to flatten`);
        const flat = join(imageHome, "qemu", `${def.id}-${key}.qcow2`);
        execFileSync("qemu-img", ["convert", "-O", "qcow2", "-c", join(server.cwd, node.runtime_overlay), flat]);
        await lab.configure(key, { hda_disk_image: flat, hda_disk_image_backing_file: null, hda_disk_image_md5sum: null });
      }
    }
    await lab.saveKeys();
    const res = await fetch(`${server.base}/v3/projects/${lab.projectId}/export?include_images=true`);
    if (!res.ok) throw new Error(`export failed: ${res.status} ${await res.text()}`);
    const bytes = Buffer.from(await res.arrayBuffer());
    writeFileSync(file, bytes);
    console.log(`  wrote ${file} (${(bytes.length / 1048576).toFixed(1)} MiB)`);
  } finally {
    await lab.stopAll().catch(() => {});
    await server.stop();
    rmSync(imageHome, { recursive: true, force: true });
  }
}

/** Import the archive into a pristine controller with an EMPTY image library and run the lab's checks. */
async function verifyArchive(def, file) {
  const imageHome = mkdtempSync(join(tmpdir(), "ns-labs-empty-images-"));
  const server = await isolatedServer({ imageHome });
  const results = [];
  const check = (name, ok) => { results.push([name, !!ok]); console.log(`  ${ok ? "PASS" : "FAIL"}  ${name}`); };
  console.log("  importing into a fresh controller with no images installed…");
  const lab = new Lab(client(server.base), "imported");
  try {
    lab.projectId = randomUUID();
    const res = await fetch(`${server.base}/v3/projects/${lab.projectId}/import?name=${encodeURIComponent(`imported ${def.id}`)}`, { method: "POST", body: readFileSync(file) });
    if (res.status !== 201) throw new Error(`import failed: ${res.status} ${await res.text()}`);
    await lab.attach();
    check("archive imported and its images installed", lab.nodes.size > 0);
    await def.verify(lab, check, { imported: true });
  } catch (error) { console.log(`  ERROR ${error.message}`); results.push(["import verify ran", false]); }
  finally {
    await lab.stopAll().catch(() => {});
    await lab.destroyProject(lab.projectId).catch(() => {});
    await server.stop();
    rmSync(imageHome, { recursive: true, force: true });
  }
  return results;
}

async function main() {
  const labs = await loadLabs();
  if (command === "list" || !command) {
    for (const lab of labs) console.log(`${lab.id.padEnd(24)} ${lab.title}\n${" ".repeat(25)}${lab.summary}`);
    return;
  }
  const chosen = pick(labs, args[0]);
  if (!chosen.length) throw new Error(`no lab matches ${args[0]}`);
  if (command === "deploy") {
    const server = flag("server", "http://127.0.0.1:3080");
    const user = flag("user", "admin");
    const passwordFile = flag("password-file", join(root, "data/initial-admin-password.txt"));
    const start = has("start");
    let token = flag("token", process.env.LABS_TOKEN);
    if (!token) try { token = await login(server, user, readFileSync(passwordFile, "utf8").trim()); } catch (error) { if (!/401|403/.test(String(error))) console.log(`(no login: ${error.message}; trying without auth)`); }
    const cli = client(server, token);
    for (const def of chosen) {
      ensureImages(def);
      const lab = new Lab(cli, `Lab ${def.id.slice(0, 2)} – ${def.title}`);
      await lab.create();
      await def.build(lab);
      console.log(`deployed ${lab.name} → ${server}/projects/${lab.projectId}`);
      if (start) { await lab.open(); await lab.startAll(); if (def.configure) await def.configure(lab); console.log("  started"); }
    }
    return;
  }
  if (command === "import-verify") {
    let failed = 0;
    for (const def of chosen) {
      console.log(`\n== import-verify ${def.id}`);
      const results = await verifyArchive(def, join(process.env.LABS_OUT ?? join(root, "labs", "projects"), `${def.id}.gns3project`));
      if (results.some(([, ok]) => !ok)) failed += 1;
    }
    process.exit(failed ? 1 : 0);
  }
  if (command === "package") {
    const outDir = process.env.LABS_OUT ?? join(root, "labs", "projects");
    mkdirSync(outDir, { recursive: true });
    let failed = 0;
    for (const def of chosen) {
      console.log(`\n== packaging ${def.id}: ${def.title}`);
      ensureImages(def);
      const file = join(outDir, `${def.id}.gns3project`);
      try {
        await packageLab(def, file);
        const results = await verifyArchive(def, file);
        if (results.some(([, ok]) => !ok)) failed += 1;
      } catch (error) { console.log(`  ERROR ${error.message}`); failed += 1; }
    }
    console.log(failed ? `\n${failed} package(s) failed` : "\nall packages built and verified after import");
    process.exit(failed ? 1 : 0);
  }
  if (command === "configure") {
    // Re-open an already deployed + started lab project by name and push its base configuration.
    const server = flag("server", "http://127.0.0.1:3080");
    const user = flag("user", "admin");
    const passwordFile = flag("password-file", join(root, "data/initial-admin-password.txt"));
    let token = flag("token", process.env.LABS_TOKEN);
    if (!token) try { token = await login(server, user, readFileSync(passwordFile, "utf8").trim()); } catch {}
    const cli = client(server, token);
    const projects = (await cli.api("GET", "/projects")).json;
    for (const def of chosen) {
      if (!def.configure) { console.log(`${def.id}: nothing to configure`); continue; }
      const name = `Lab ${def.id.slice(0, 2)} – ${def.title}`;
      const project = projects.find((p) => p.name === name);
      if (!project) throw new Error(`deploy ${def.id} first`);
      const lab = new Lab(cli, name); lab.projectId = project.project_id;
      const nodes = (await cli.api("GET", `/projects/${lab.projectId}/nodes`)).json;
      for (const n of nodes) lab.nodes.set(n.name.toLowerCase(), { ...n, api_base: server });
      await def.attach?.(lab);
      await def.configure(lab);
      console.log(`configured ${name}`);
    }
    return;
  }
  if (command === "verify") {
    let failed = 0;
    const keep = has("keep");
    for (const def of chosen) {
      console.log(`\n== ${def.id}: ${def.title}`);
      ensureImages(def);
      const server = await isolatedServer();
      const lab = new Lab(client(server.base), `verify-${def.id}`);
      const results = [];
      const check = (name, ok) => { results.push([name, !!ok]); console.log(`  ${ok ? "PASS" : "FAIL"}  ${name}`); };
      try {
        await lab.create();
        await def.build(lab);
        await def.verify(lab, check);
      } catch (error) { console.log(`  ERROR ${error.message}`); results.push(["lab ran", false]); }
      finally {
        if (keep) { console.log(`  --keep: server ${server.base} project ${lab.projectId}; waiting for /tmp/labs-release`); while (!existsSync("/tmp/labs-release")) await new Promise((r) => setTimeout(r, 1000)); rmSync("/tmp/labs-release"); }
        await lab.stopAll().catch(() => {});
        await lab.destroyProject(lab.projectId).catch(() => {});
        await server.stop();
      }
      if (results.some(([, ok]) => !ok)) failed += 1;
    }
    console.log(failed ? `\n${failed} lab(s) failed` : "\nall labs passed");
    process.exit(failed ? 1 : 0);
  }
  throw new Error(`unknown command ${command}`);
}
main().catch((error) => { console.error(error.message); process.exit(1); });
