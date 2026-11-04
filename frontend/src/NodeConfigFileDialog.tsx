import { useEffect, useRef, useState } from "react";
import { Download, Upload } from "lucide-react";
import type { Node } from "./api";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Textarea } from "@/components/ui/textarea";

/** The node property holding the editable startup configuration, as in GNS3's Edit config action. */
export function configFieldForNode(node: Node): "startup_script" | "startup_config" | null {
  if (node.node_type === "vpcs") return "startup_script";
  if (node.node_type === "dynamips" || node.node_type === "iou") return "startup_config";
  return null;
}

export function configFileName(node: Node): string {
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

export function NodeConfigFileDialog({ node, open, onOpenChange, onSave }: {
  node: Node | null; open: boolean; onOpenChange: (open: boolean) => void;
  onSave: (id: string, patch: Record<string, unknown>) => Promise<void>;
}) {
  const field = node ? configFieldForNode(node) : null;
  const [text, setText] = useState("");
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  useEffect(() => { if (open && node && field) { setText(String(node[field] ?? "")); setError(""); } }, [open, node, field]);
  if (!node || !field) return null;
  const save = async () => {
    setSaving(true); setError("");
    try { await onSave(node.node_id, { [field]: text }); onOpenChange(false); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "Could not save the configuration. Stop the device first."); }
    finally { setSaving(false); }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Edit config · {node.name}</DialogTitle>
          <DialogDescription>{field === "startup_script" ? "VPCS startup script, run when the device starts." : "Startup configuration written before the device starts."} The device must be stopped to save.</DialogDescription>
        </DialogHeader>
        <Textarea aria-label="Configuration" className="min-h-72 font-mono text-xs" value={text} onChange={(event) => setText(event.target.value)} />
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        <input ref={fileInput} type="file" className="hidden" aria-label="Import configuration file" onChange={async (event) => { const file = event.target.files?.[0]; if (file) setText(await file.text()); event.target.value = ""; }} />
        <DialogFooter className="gap-2">
          <Button type="button" variant="outline" onClick={() => fileInput.current?.click()}><Upload />Import from file</Button>
          <Button type="button" variant="outline" onClick={() => { const url = URL.createObjectURL(new Blob([text], { type: "text/plain" })); const link = document.createElement("a"); link.href = url; link.download = configFileName(node); link.click(); URL.revokeObjectURL(url); }}><Download />Export to file</Button>
          <Button type="button" disabled={saving} onClick={() => void save()}>Save</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
