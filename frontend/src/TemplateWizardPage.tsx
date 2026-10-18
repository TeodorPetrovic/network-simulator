import { useMemo, useState } from "react";
import type { FormEvent } from "react";
import { useNavigate } from "@tanstack/react-router";
import { ArrowLeft, ArrowRight, Check, Download, FileUp, Search } from "lucide-react";
import type { Gns3Appliance, Gns3ApplianceSearchResult } from "./api";
import { api } from "./api";
import { PagesShell, PageTitle } from "./PagesShell";
import { TemplateEditor } from "./PreferencesPage";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

type WizardMode = "choose" | "manual" | "install";
type RuntimeKey = "docker" | "qemu" | "vpcs" | "dynamips" | "cloud" | "nat" | "ethernet_hub" | "ethernet_switch";
const runtimes: Array<{ key: RuntimeKey; label: string }> = [
  { key: "docker", label: "Docker" }, { key: "qemu", label: "QEMU" }, { key: "vpcs", label: "VPCS" }, { key: "dynamips", label: "Dynamips (IOS router)" },
  { key: "cloud", label: "Cloud" }, { key: "nat", label: "NAT" }, { key: "ethernet_hub", label: "Ethernet hub" }, { key: "ethernet_switch", label: "Ethernet switch" },
];
const errorMessage = (reason: unknown) => reason instanceof Error ? reason.message : "Request failed";

export function TemplateWizardPage() {
  const navigate = useNavigate();
  const [mode, setMode] = useState<WizardMode>("choose");
  const [runtime, setRuntime] = useState<RuntimeKey>("docker");
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("all");
  const [results, setResults] = useState<Gns3ApplianceSearchResult[]>([]);
  const [selected, setSelected] = useState<Gns3Appliance | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [file, setFile] = useState<File | null>(null);
  const [creating, setCreating] = useState(false);

  const categories = useMemo(() => ["all", ...new Set(results.map((item) => item.category).filter((value): value is string => Boolean(value)))], [results]);
  const visible = results.filter((item) => category === "all" || item.category === category);

  async function searchRegistry(event?: FormEvent<HTMLFormElement>) {
    event?.preventDefault(); setBusy(true); setError(""); setSelected(null);
    try { setResults(await api.searchGns3Appliances(search.trim())); }
    catch (reason) { setError(`Appliance registry is unavailable: ${errorMessage(reason)}`); setResults([]); }
    finally { setBusy(false); }
  }
  async function inspect(item: Gns3ApplianceSearchResult) {
    setBusy(true); setError("");
    try { setSelected(await api.gns3Appliance(item.file_name)); }
    catch (reason) { setError(`Appliance details are unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  async function importSelected() {
    if (!selected) return;
    setBusy(true); setError(""); setNotice("");
    try { await api.importGns3Appliance(selected.file_name); setNotice(`Imported “${selected.name}” definition. Required images still need explicit local bindings before its template is ready.`); }
    catch (reason) { setError(`Appliance import is unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  async function importFile() {
    if (!file) return;
    setBusy(true); setError(""); setNotice("");
    try { const appliance = await api.importGns3ApplianceFile(file); setNotice(`Imported “${appliance.name}” definition. Review its image requirements in the appliance library before creating a node template.`); }
    catch (reason) { setError(`Appliance file import is unavailable: ${errorMessage(reason)}`); }
    finally { setBusy(false); }
  }
  async function createManual(patch: Record<string, unknown>) {
    const created = await api.createTemplate({ ...patch, template_type: runtime, name: String(patch.name ?? "New template") });
    setCreating(false); setNotice(`Created “${created.name}”.`);
  }
  function beginInstall() { setMode("install"); if (!results.length) void searchRegistry(); }

  return <PagesShell active="Preferences"><div className="pages-content">
    <PageTitle eyebrow="TEMPLATE SETUP" title="New template wizard" description="Add a runtime template manually or import an appliance definition from the GNS3 registry or a local file." actions={<Button variant="outline" onClick={() => void navigate({ to: "/preferences" as never })}><ArrowLeft size={15} />Preferences</Button>} />
    {error && <div role="alert" className="pages-alert">{error}</div>}{notice && <div role="status" className="pages-notice">{notice}<Button variant="ghost" size="sm" onClick={() => void navigate({ to: "/appliances" as never })}>Open appliance library</Button></div>}

    {mode === "choose" && <div className="wizard-choice-grid"><Card className="wizard-choice" role="button" tabIndex={0} onClick={() => setMode("manual")} onKeyDown={(event) => event.key === "Enter" && setMode("manual")}><div className="wizard-choice-icon"><Check size={19} /></div><h2>Manual template</h2><p>Choose a runtime and configure the fields this controller currently supports.</p><Button onClick={(event) => { event.stopPropagation(); setMode("manual"); }}>Continue <ArrowRight size={15} /></Button></Card><Card className="wizard-choice" role="button" tabIndex={0} onClick={beginInstall} onKeyDown={(event) => event.key === "Enter" && beginInstall()}><div className="wizard-choice-icon"><Download size={19} /></div><h2>Install appliance</h2><p>Import an appliance definition from the GNS3 registry or upload a .gns3a file.</p><Button variant="outline" onClick={(event) => { event.stopPropagation(); beginInstall(); }}>Browse appliances <ArrowRight size={15} /></Button></Card></div>}

    {mode === "manual" && <Card className="wizard-panel"><div className="wizard-panel-header"><div><h2>Manual template</h2><p>Select a runtime to open its grouped settings form.</p></div><Button variant="ghost" onClick={() => setMode("choose")}><ArrowLeft size={15} />Back</Button></div><label className="grid max-w-lg gap-1.5"><Label htmlFor="manual-runtime">Runtime</Label><Select value={runtime} onValueChange={(value) => setRuntime(value as RuntimeKey)}><SelectTrigger id="manual-runtime"><SelectValue /></SelectTrigger><SelectContent>{runtimes.map((item) => <SelectItem key={item.key} value={item.key}>{item.label}</SelectItem>)}</SelectContent></Select><span className="text-xs text-muted-foreground">Fields tagged Pending backend are shown for parity and will not be sent.</span></label><div className="mt-5 flex gap-2"><Button onClick={() => setCreating(true)}><ArrowRight size={15} />Configure template</Button><Button variant="outline" onClick={() => void navigate({ to: "/preferences" as never })}>Use existing templates</Button></div></Card>}

    {mode === "install" && <Card className="wizard-panel"><div className="wizard-panel-header"><div><h2>Install appliance</h2><p>Registry definitions can be imported. Image downloads and verification remain explicit in the appliance library.</p></div><Button variant="ghost" onClick={() => setMode("choose")}><ArrowLeft size={15} />Back</Button></div><Tabs defaultValue="registry"><TabsList><TabsTrigger value="registry">GNS3 registry</TabsTrigger><TabsTrigger value="file">Appliance file</TabsTrigger></TabsList><TabsContent value="registry" className="grid gap-4 pt-4"><form className="registry-search" onSubmit={(event) => void searchRegistry(event)}><label className="grid min-w-0 flex-1 gap-1.5"><Label htmlFor="registry-search">Filter appliances</Label><Input id="registry-search" placeholder="Search by name" value={search} onChange={(event) => setSearch(event.target.value)} /></label><label className="grid w-56 gap-1.5"><Label htmlFor="registry-category">Category</Label><Select value={category} onValueChange={setCategory}><SelectTrigger id="registry-category"><SelectValue /></SelectTrigger><SelectContent>{categories.map((item) => <SelectItem key={item} value={item}>{item === "all" ? "All categories" : item}</SelectItem>)}</SelectContent></Select></label><Button type="submit" disabled={busy}><Search size={15} />Search</Button></form><div className="registry-results">{busy && !results.length ? <p className="p-4 text-sm text-muted-foreground">Searching the GNS3 registry…</p> : visible.length ? visible.map((item) => <button key={item.file_name} className={`registry-result${selected?.file_name === item.file_name ? " is-selected" : ""}`} onClick={() => void inspect(item)}><span><strong>{item.name}</strong><small>{item.category ?? "Uncategorized"} · {item.file_name}</small></span><Badge variant={item.imported ? "secondary" : "outline"}>{item.imported ? "Imported" : "Available"}</Badge></button>) : <p className="p-4 text-sm text-muted-foreground">No registry appliances match this filter.</p>}</div>{selected && <div className="appliance-inspection"><div><h3>{selected.name}</h3><p>{selected.description || "No description supplied."}</p></div><dl><dt>Vendor / maintainer</dt><dd>{selected.template_type ?? "Unknown runtime"}</dd><dt>Category</dt><dd>{selected.category ?? "Uncategorized"}</dd><dt>Status</dt><dd>{selected.status ?? "No status supplied"}</dd><dt>Required files</dt><dd>{selected.required_files?.length ?? 0}</dd></dl><Button onClick={() => void importSelected()} disabled={busy || selected.imported}>{busy ? "Importing…" : selected.imported ? "Already imported" : "Import definition"}</Button></div>}</TabsContent><TabsContent value="file" className="grid max-w-xl gap-3 pt-4"><label className="grid gap-1.5"><Label htmlFor="appliance-file">Appliance definition (.gns3a / .gns3appliance)</Label><Input id="appliance-file" type="file" accept=".gns3a,.gns3appliance" onChange={(event) => setFile(event.target.files?.[0] ?? null)} /></label><p className="text-xs text-muted-foreground">The controller imports the definition only. Required runtime images are not installed or synthesized.</p><Button className="w-fit" onClick={() => void importFile()} disabled={!file || busy}><FileUp size={15} />{busy ? "Importing…" : "Import appliance definition"}</Button></TabsContent></Tabs></Card>}

    <TemplateEditor runtime={runtime} open={creating} onOpenChange={setCreating} onSave={createManual} creating />
  </div></PagesShell>;
}
