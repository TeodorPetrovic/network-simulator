import type { FormEvent } from "react";
import { api } from "./api";
import type { RuntimeTemplate, TopologySnapshot } from "./api";
import { errorMessage, upsertNode } from "./appUtils";

type Args = {
  projectId: string | null; topology: TopologySnapshot | null; nodeName: string; templateId: string; nodeCommand: string; nodeStartupScript: string; nodeStartupConfig: string; nodeIdlePc: string;
  setCreatingNode: (value: boolean) => void; setTopology: (update: (current: TopologySnapshot | null) => TopologySnapshot | null) => void;
  setSelection: (value: { kind: "node" | "link"; id: string } | null) => void; setNodeName: (update: string | ((current: string) => string)) => void;
  setNodeDialogOpen: (value: boolean) => void; setNotice: (value: string) => void; setError: (value: string) => void;
  setLoadingTemplates: (value: boolean) => void; setTemplates: (value: RuntimeTemplate[]) => void; setTemplateId: (value: string) => void;
  setNodeCommand: (value: string) => void; setNodeStartupScript: (value: string) => void; setNodeStartupConfig: (value: string) => void; setNodeIdlePc: (value: string) => void;
  setUploadingImage: (value: boolean) => void; setUploadingQemuImage: (value: boolean) => void;
};

export function useAppNodeActions({ projectId, topology, nodeName, templateId, nodeCommand, nodeStartupScript, nodeStartupConfig, nodeIdlePc, setCreatingNode, setTopology, setSelection, setNodeName, setNodeDialogOpen, setNotice, setError, setLoadingTemplates, setTemplates, setTemplateId, setNodeCommand, setNodeStartupScript, setNodeStartupConfig, setNodeIdlePc, setUploadingImage, setUploadingQemuImage }: Args) {
  async function addNode(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!projectId || !topology || !nodeName.trim() || !templateId) return;
    const index = topology.nodes.length;
    try {
      setCreatingNode(true);
      const node = await api.createNodeFromTemplate(projectId, templateId, {
        name: nodeName.trim(),
        x: -420 + (index % 4) * 270,
        y: -210 + Math.floor(index / 4) * 170,
        ...(nodeCommand.trim() ? { start_command: nodeCommand.trim() } : {}),
        ...(nodeStartupScript.trim() ? { startup_script: nodeStartupScript } : {}),
        ...(nodeIdlePc.trim() ? { idlepc: nodeIdlePc.trim() } : {}),
        ...(nodeStartupConfig.trim() ? { startup_config: nodeStartupConfig } : {}),
      });
      setTopology((current) => current ? { ...current, nodes: upsertNode(current.nodes, node) } : current);
      setSelection({ kind: "node", id: node.node_id });
      setNodeName("");
      setNodeDialogOpen(false);
      const kind = node.node_type === "dynamips" ? "Cisco IOS router" : node.node_type === "qemu" ? "virtual machine" : node.node_type === "vpcs" ? "VPCS" : "container";
      setNotice(`${node.name} ${kind} created and added to the topology`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setCreatingNode(false);
    }
  }

  async function openNodeDialog() {
    setNodeDialogOpen(true);
    setLoadingTemplates(true);
    try {
      const available = await api.templates();
      setTemplates(available);
      const selected = available.find((item) => item.template_id === templateId) ?? available[0];
      setTemplateId(selected?.template_id ?? "");
      setNodeCommand(selected?.template_type === "docker" ? selected.start_command ?? "" : "");
      setNodeStartupScript(selected?.template_type === "vpcs" ? selected.startup_script ?? "" : "");
      setNodeStartupConfig("");
      setNodeIdlePc(selected?.template_type === "dynamips" ? selected.idlepc : "");
    } catch (reason) {
      setTemplates([]);
      setTemplateId("");
      setError(errorMessage(reason));
    } finally {
      setLoadingTemplates(false);
    }
  }

  async function importDynamipsImage(file?: File) {
    if (!file) return;
    setUploadingImage(true);
    setError("");
    try {
      const uploaded = await api.uploadDynamipsImage(file);
      const available = await api.templates();
      setTemplates(available);
      const template = available.find((item) => item.template_type === "dynamips" && item.image === uploaded.image);
      if (template?.template_type === "dynamips") {
        setTemplateId(template.template_id);
        setNodeName((current) => current || file.name.replace(/\.[^.]+$/, ""));
        setNodeIdlePc(template.idlepc);
      }
      setNotice(`Imported IOS image ${uploaded.name} (${(uploaded.bytes / (1024 * 1024)).toFixed(1)} MiB)`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setUploadingImage(false);
    }
  }

  async function importQemuImage(file?: File) {
    if (!file) return;
    setUploadingQemuImage(true);
    setError("");
    try {
      const uploaded = await api.uploadQemuImage(file);
      const available = await api.templates();
      setTemplates(available);
      const template = available.find((item) => item.template_type === "qemu" && item.hda_disk_image === uploaded.image);
      if (template?.template_type === "qemu") {
        setTemplateId(template.template_id);
        setNodeName((current) => current || file.name.replace(/\.[^.]+$/, ""));
      }
      setNotice(`Added QEMU image ${uploaded.name} (${(uploaded.bytes / (1024 * 1024)).toFixed(1)} MiB) to the global image library`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setUploadingQemuImage(false);
    }
  }


  return { addNode, openNodeDialog, importDynamipsImage, importQemuImage };
}
