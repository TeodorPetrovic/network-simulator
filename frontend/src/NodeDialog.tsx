import type { FormEvent } from "react";
import type { RuntimeInfo, RuntimeTemplate } from "./api";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";

type NodeDialogProps = {
  open: boolean; onOpenChange: (open: boolean) => void; onSubmit: (event: FormEvent<HTMLFormElement>) => void;
  nodeName: string; setNodeName: (value: string) => void; templates: RuntimeTemplate[]; templateId: string; setTemplateId: (value: string) => void;
  setNodeCommand: (value: string) => void; setNodeStartupScript: (value: string) => void; setNodeStartupConfig: (value: string) => void; setNodeIdlePc: (value: string) => void;
  nodeCommand: string; nodeStartupScript: string; nodeStartupConfig: string; nodeIdlePc: string; runtimeInfo: RuntimeInfo | null;
  loadingTemplates: boolean; creatingNode: boolean; uploadingImage: boolean; uploadingQemuImage: boolean;
  onImportImage: (file?: File) => void; onImportQemuImage: (file?: File) => void;
};

export function NodeDialog({ open, onOpenChange, onSubmit, nodeName, setNodeName, templates, templateId, setTemplateId, setNodeCommand, setNodeStartupScript, setNodeStartupConfig, setNodeIdlePc, nodeCommand, nodeStartupScript, nodeStartupConfig, nodeIdlePc, runtimeInfo, loadingTemplates, creatingNode, uploadingImage, uploadingQemuImage, onImportImage, onImportQemuImage }: NodeDialogProps) {
  const selectedNodeTemplate = templates.find((item) => item.template_id === templateId);
  return (
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent className="max-h-[92dvh] max-w-xl overflow-y-auto">
          <form onSubmit={onSubmit} className="space-y-4">
            <DialogHeader><DialogTitle>Add a node</DialogTitle><DialogDescription>Choose an installed template or add an image to the global library.</DialogDescription></DialogHeader>
            <div className="grid gap-2"><Label htmlFor="node-name">Node name</Label><Input id="node-name" autoFocus placeholder="e.g. Core router" value={nodeName} onChange={(event) => setNodeName(event.target.value)} /></div>
            <div className="grid gap-2"><Label htmlFor="node-template">Available device template</Label>
              {loadingTemplates ? <Card className="p-3 text-sm text-muted-foreground" role="status">Discovering local runtime images…</Card> : templates.length ? (
                <Select value={templateId} onValueChange={(value) => {
                  const selected = templates.find((item) => item.template_id === value);
                  setTemplateId(value);
                  setNodeCommand(selected?.template_type === "docker" ? selected.start_command ?? "" : "");
                  setNodeStartupScript(selected?.template_type === "vpcs" ? selected.startup_script ?? "" : "");
                  setNodeStartupConfig("");
                  setNodeIdlePc(selected?.template_type === "dynamips" ? selected.idlepc : "");
                }}>
                  <SelectTrigger id="node-template"><SelectValue placeholder="Select a device template" /></SelectTrigger>
                  <SelectContent>{templates.map((template) => <SelectItem key={template.template_id} value={template.template_id}>{template.template_type === "dynamips" ? "Dynamips / Cisco IOS · " : template.template_type === "qemu" ? "QEMU · " : template.template_type === "vpcs" ? "VPCS · " : template.template_type === "docker" ? "Docker · " : `${template.template_type.replaceAll("_", " ")} · `}{template.name}</SelectItem>)}</SelectContent>
                </Select>
              ) : <Card className="p-3 text-sm text-muted-foreground" role="status">{runtimeInfo?.docker.available || runtimeInfo?.qemu.available || runtimeInfo?.vpcs.available || runtimeInfo?.dynamips.available ? <>No local templates yet. Pull a Docker image, add a QEMU disk under <code>{runtimeInfo.qemu.image_directory || "images/qemu"}</code>, install VPCS, or import a Cisco IOS image below.</> : <>Docker Engine, QEMU, VPCS, and Dynamips are unavailable. Check the controller service account's runtime access.</>}</Card>}
            </div>
            <Card className="grid gap-2 p-3 shadow-none"><Label htmlFor="qemu-image-upload">Add a QEMU disk image</Label>
              <Input id="qemu-image-upload" type="file" accept=".qcow2,.qcow,.raw,.img,.vmdk,.vdi" disabled={uploadingQemuImage} onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; void onImportQemuImage(file); }} />
              <p className="text-xs text-muted-foreground">{uploadingQemuImage ? "Uploading and validating with qemu-img…" : `Validated with qemu-img, then stored globally in ${runtimeInfo?.qemu.image_directory || "the QEMU image directory"}. Maximum upload size: 64 GiB.`}</p>
            </Card>
            <Card className="grid gap-2 p-3 shadow-none"><Label htmlFor="ios-image-upload">Import Cisco IOS for Dynamips</Label>
              <Input id="ios-image-upload" type="file" accept=".bin,.image,.img,.elf" disabled={uploadingImage} onChange={(event) => { const file = event.currentTarget.files?.[0]; event.currentTarget.value = ""; void onImportImage(file); }} />
              <p className="text-xs text-muted-foreground">{uploadingImage ? "Uploading and validating the IOS ELF image…" : `Validated as a Dynamips-compatible ELF image, then stored in ${runtimeInfo?.dynamips.image_directory || "images/dynamips"}.`}</p>
            </Card>
            {templates.find((item) => item.template_id === templateId)?.template_type === "docker" && <div className="grid gap-2"><Label htmlFor="node-command">Container command (optional)</Label><Input id="node-command" placeholder="e.g. /bin/sh -c 'sleep 3600'" value={nodeCommand} onChange={(event) => setNodeCommand(event.target.value)} /></div>}
            {templates.find((item) => item.template_id === templateId)?.template_type === "vpcs" && <div className="grid gap-2"><Label htmlFor="vpcs-startup-script">Startup script (optional)</Label><Textarea id="vpcs-startup-script" rows={4} placeholder="ip 192.168.1.10/24" value={nodeStartupScript} onChange={(event) => setNodeStartupScript(event.target.value)} /></div>}
            {selectedNodeTemplate?.template_type === "dynamips" && <><div className="grid gap-2"><Label htmlFor="dynamips-idlepc">Idle-PC address (optional)</Label><Input id="dynamips-idlepc" placeholder="Use the appliance default" value={nodeIdlePc} onChange={(event) => setNodeIdlePc(event.target.value)} /><p className="text-xs text-muted-foreground">The imported appliance value is used when present. Auto-selected values are saved on the router and reused after restart.</p></div><div className="grid gap-2"><Label htmlFor="dynamips-startup-config">IOS startup configuration (optional)</Label><Textarea id="dynamips-startup-config" rows={5} placeholder="hostname R1&#10;no ip domain-lookup" value={nodeStartupConfig} onChange={(event) => setNodeStartupConfig(event.target.value)} /></div></>}
            {selectedNodeTemplate?.template_type === "qemu" && <Card className="p-3 text-xs text-muted-foreground shadow-none"><b className="text-foreground">QEMU virtual machine</b><p className="mt-1">Creates a private copy-on-write disk. {runtimeInfo?.qemu.kvm_accessible ? "KVM acceleration is available." : runtimeInfo?.qemu.acceleration_error ?? "QEMU may use TCG if KVM access is unavailable."}</p></Card>}
            {selectedNodeTemplate?.template_type === "vpcs" && <Card className="p-3 text-xs text-muted-foreground shadow-none"><b className="text-foreground">Virtual PC simulator</b><p className="mt-1">Starts a lightweight VPCS process with one Ethernet adapter and an interactive console.</p></Card>}
            {selectedNodeTemplate?.template_type === "dynamips" && <Card className="p-3 text-xs text-muted-foreground shadow-none"><b className="text-foreground">Cisco IOS router · Dynamips c7200</b><p className="mt-1">Creates a Dynamips hypervisor router from the imported IOS image.</p></Card>}
            {selectedNodeTemplate?.template_type === "docker" && <Card className="p-3 text-xs text-muted-foreground shadow-none"><b className="text-foreground">Docker container</b><p className="mt-1">Creates a stopped container from the selected local image.</p></Card>}
            <div className="flex justify-end gap-2"><Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button><Button type="submit" disabled={!nodeName.trim() || !templateId || loadingTemplates || creatingNode || uploadingImage || uploadingQemuImage}>{creatingNode ? "Creating…" : "Create node"}</Button></div>
          </form>
        </DialogContent>
      </Dialog>
  );
}
