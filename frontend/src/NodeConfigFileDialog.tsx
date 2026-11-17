import { useEffect, useRef, useState } from "react";
import { Download, Upload } from "lucide-react";
import type { Node } from "./api";
import { api } from "./api";
import { Button } from "@/components/ui/button";
import { SettingsDialog, SettingsNotice, SettingsSection } from "@/components/settings-dialog";
import { Textarea } from "@/components/ui/textarea";

/** The node property holding the editable startup configuration, as in GNS3's Edit config action. */
export function configFieldForNode(node: Node): "startup_script" | "startup_config" | null {
  if (node.node_type === "vpcs") return "startup_script";
  if (node.node_type === "dynamips" || node.node_type === "iou") return "startup_config";
  return null;
}

/** Docker nodes keep their network configuration (ifupdown) as a file applied when the container boots. */
export const DOCKER_NETWORK_FILE = "etc/network/interfaces";
export const isDockerNetworkConfig = (node: Node) => node.node_type === "docker";
export const canEditNodeConfig = (node: Node) => configFieldForNode(node) !== null || isDockerNetworkConfig(node);

export function defaultNetworkConfig(node: Node): string {
  const adapters = Math.min(Number(node.adapters ?? 4) || 4, 16);
  const header = "#\n# This is a sample network config, please uncomment lines to configure the network\n#\n\n# Uncomment this line to load custom interface files\n# source /etc/network/interfaces.d/*\n";
  const blocks = Array.from({ length: adapters }, (_, i) => `\n# Static config for eth${i}\n#auto eth${i}\n#iface eth${i} inet static\n#\taddress 192.168.${i + 1}.2\n#\tnetmask 255.255.255.0\n#\tgateway 192.168.${i + 1}.1\n#\tup echo nameserver 192.168.${i + 1}.1 > /etc/resolv.conf\n\n# DHCP config for eth${i}\n#auto eth${i}\n#iface eth${i} inet dhcp\n#\thostname ${node.name}\n`);
  return header + blocks.join("");
}

export function configFileName(node: Node): string {
  if (isDockerNetworkConfig(node)) return "interfaces";
  return `${node.name}_startup.${node.node_type === "vpcs" ? "vpc" : "cfg"}`;
}

export function downloadNodeConfig(node: Node) {
  const field = configFieldForNode(node);
  if (!field) return;
  const url = URL.createObjectURL(new Blob([String(node[field] ?? "")], { type: "text/plain" }));
  const link = document.createElement("a");
  link.href = url;
  link.download = configFileName(node);
  link.click();
  URL.revokeObjectURL(url);
}

export function NodeConfigFileDialog({ node, projectId, open, onOpenChange, onSave }: {
  node: Node | null; projectId?: string; open: boolean; onOpenChange: (open: boolean) => void;
  onSave: (id: string, patch: Record<string, unknown>) => Promise<void>;
}) {
  const field = node ? configFieldForNode(node) : null;
  const docker = node ? isDockerNetworkConfig(node) : false;
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (!open || !node) return;
    setError("");
    if (field) { setText(String(node[field] ?? "")); return; }
    if (!docker || !projectId) return;
    let cancelled = false;
    setText("");
    api.readNodeFile(projectId, node.node_id, DOCKER_NETWORK_FILE)
      .then((content) => { if (!cancelled) setText(content ?? defaultNetworkConfig(node)); })
      .catch((reason) => { if (!cancelled) { setText(defaultNetworkConfig(node)); setError(reason instanceof Error ? reason.message : "Could not load the saved configuration."); } });
    return () => { cancelled = true; };
  }, [open, node, field, docker, projectId]);
  if (!node || (!field && !docker)) return null;
  const save = async () => {
    setSaving(true); setError("");
    try {
      if (docker) { if (!projectId) throw new Error("Open the device from a project to edit its network configuration."); await api.writeNodeFile(projectId, node.node_id, DOCKER_NETWORK_FILE, text); }
      else if (field) await onSave(node.node_id, { [field]: text });
      onOpenChange(false);
    }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Could not save the configuration. Stop the device first."); }
    finally { setSaving(false); }
  };
  return <SettingsDialog open={open} onOpenChange={onOpenChange} title={`Edit config · ${node.name}`} description={docker ? "Network configuration (ifupdown) applied inside the container each time it starts or reloads." : field === "startup_script" ? "VPCS startup script, run when the device starts." : "Startup configuration written before the device starts."} onSubmit={(event) => { event.preventDefault(); void save(); }} saving={saving} saveDisabled={!docker && node.status !== "stopped"} saveLabel="Save" error={error} notice={docker ? (node.status === "started" ? <SettingsNotice>Changes apply the next time this device starts or reloads. Reload it to apply them now.</SettingsNotice> : undefined) : node.status !== "stopped" ? <SettingsNotice>Stop this device before saving its startup configuration.</SettingsNotice> : undefined}>
    <SettingsSection title={docker ? "Network configuration" : "Startup configuration"}>
      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border bg-muted/30 p-3">
        <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">{configFileName(node)}</span>
        <div className="flex flex-wrap gap-2">
          <Button type="button" size="sm" variant="outline" onClick={() => fileInput.current?.click()}><Upload />Import from file</Button>
          <Button type="button" size="sm" variant="outline" onClick={() => { const url = URL.createObjectURL(new Blob([text], { type: "text/plain" })); const link = document.createElement("a"); link.href = url; link.download = configFileName(node); link.click(); URL.revokeObjectURL(url); }}><Download />Export to file</Button>
        </div>
      </div>
      <Textarea aria-label="Configuration" className="min-h-64 resize-y font-mono text-xs leading-relaxed" rows={16} spellCheck={false} value={text} onChange={(event) => setText(event.target.value)} />
      <input ref={fileInput} type="file" className="hidden" aria-label="Import configuration file" onChange={async (event) => { const input = event.currentTarget; const file = input.files?.[0]; try { if (file) setText(await file.text()); } catch { setError("Could not read the configuration file."); } finally { input.value = ""; } }} />
    </SettingsSection>
  </SettingsDialog>;
}
