import type { Dispatch, SetStateAction } from "react";
import type { ConsoleTarget } from "./ConsoleDock";
import { api } from "./api";
import type { Node, RuntimeTemplate, TopologySnapshot } from "./api";
import { errorMessage, generatedNodeName, upsertNode } from "./appUtils";

type Args = {
  projectId: string | null; topology: TopologySnapshot | null; templates: RuntimeTemplate[];
  setError: (value: string) => void; setNotice: (value: string) => void;
  setTopology: (update: (current: TopologySnapshot | null) => TopologySnapshot | null) => void;
  setSelection: (value: { kind: "node" | "link"; id: string } | null) => void;
  setConsoleTargets: Dispatch<SetStateAction<ConsoleTarget[]>>;
  pendingNodeNames: Set<string>; setTemplates: (update: (current: RuntimeTemplate[]) => RuntimeTemplate[]) => void;
};

export function useAppDeviceActions({ projectId, topology, templates, setError, setNotice, setTopology, setSelection, setConsoleTargets, pendingNodeNames, setTemplates }: Args) {
  async function addTemplateAt(templateId: string, x?: number, y?: number) {
    if (!projectId || !topology) return;
    const template = templates.find((item) => item.template_id === templateId);
    if (!template) {
      setError("That device template is no longer available. Refresh the device library and try again.");
      return;
    }
    const index = topology.nodes.length;
    const position = { x: x ?? -420 + (index % 5) * 180, y: y ?? -220 + Math.floor(index / 5) * 150 };
    const name = template.template_type === "dynamips" ? generatedNodeName(template, topology.nodes, pendingNodeNames).replace(/[^a-zA-Z0-9._-]/g, "") : generatedNodeName(template, topology.nodes, pendingNodeNames);
    pendingNodeNames.add(name);
    try {
      const node = await api.createNodeFromTemplate(projectId, templateId, {
        name,
        ...position,
        ...(template.template_type === "docker" && template.start_command ? { start_command: template.start_command } : {}),
        ...(template.template_type === "vpcs" && template.startup_script ? { startup_script: template.startup_script } : {}),
        ...(template.template_type === "dynamips" ? { idlepc: template.idlepc, mmap: template.mmap ?? true, sparsemem: template.sparsemem ?? true, auto_delete_disks: template.auto_delete_disks ?? false, ghost_ios: template.ghost_ios ?? false, console_auto_start: template.console_auto_start ?? false } : {}),
      });
      setTopology((current) => current ? { ...current, nodes: upsertNode(current.nodes, node) } : current);
      setSelection({ kind: "node", id: node.node_id });
      setNotice(`${node.name} added to the topology`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      pendingNodeNames.delete(name);
    }
  }

  async function renameNode(nodeId: string, name: string) {
    if (!projectId) return;
    try {
      const node = await api.updateNode(projectId, nodeId, { name });
      setTopology((current) => current ? { ...current, nodes: current.nodes.map((item) => item.node_id === nodeId ? node : item) } : current);
      setConsoleTargets((current) => current.map((target) => target.nodeId === nodeId ? { ...target, nodeName: node.name } : target));
      setNotice(`Device renamed to ${node.name}`);
    } catch (reason) {
      setError(errorMessage(reason));
      throw reason;
    }
  }

  async function duplicateNode(source: Node) {
    if (!projectId || !topology) return;
    let reservedName: string | null = null;
    try {
      const payload: Record<string, unknown> = { ...source };
      for (const key of [
        "node_id", "project_id", "container_id", "image_digest", "status", "locked",
        "runtime_missing", "runtime_overlay", "runtime_qmp_socket", "runtime_serial_socket",
        "runtime_pid_file", "runtime_log_file", "console_host", "console_port",
      ]) delete payload[key];
      const baseName = `${source.name} copy`;
      const usedNames = new Set(topology.nodes.map((node) => node.name));
      let name = baseName;
      let suffix = 2;
      while (usedNames.has(name) || pendingNodeNames.has(name)) name = `${baseName} ${suffix++}`;
      reservedName = name;
      payload.name = name;
      payload.x = Number(source.x ?? 0) + 140;
      payload.y = Number(source.y ?? 0) + 100;
      pendingNodeNames.add(name);
      const node = await api.createNode(projectId, payload as Omit<Node, "node_id">);
      setTopology((current) => current ? { ...current, nodes: upsertNode(current.nodes, node) } : current);
      setSelection({ kind: "node", id: node.node_id });
      setNotice(`${node.name} duplicated`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      if (reservedName) pendingNodeNames.delete(reservedName);
    }
  }

  async function configureNode(nodeId: string, patch: Record<string, unknown>) {
    if (!projectId) return;
    try {
      const node = await api.configureNode(projectId, nodeId, patch);
      setTopology((current) => current ? { ...current, nodes: current.nodes.map((item) => item.node_id === nodeId ? node : item) } : current);
      setConsoleTargets((current) => current.map((target) => target.nodeId === nodeId ? { ...target, nodeName: node.name } : target));
      setNotice(`${node.name} device settings saved`);
    } catch (reason) {
      setError(errorMessage(reason));
      throw reason;
    }
  }

  async function saveTemplateSettings(templateId: string, patch: Record<string, unknown>) {
    try {
      const updated = await api.updateTemplate(templateId, patch);
      setTemplates((current) => current.map((template) => template.template_id === templateId ? updated : template));
      setNotice(`${updated.name} template settings saved.`);
    } catch (reason) {
      setError(errorMessage(reason));
      throw reason;
    }
  }


  return { addTemplateAt, renameNode, duplicateNode, configureNode, saveTemplateSettings };
}
