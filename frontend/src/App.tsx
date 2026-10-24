import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { Activity, Network } from "lucide-react";
import { api, dockerHttpConsoleUrl } from "./api";
import type { LinkEndpoint, Node, Project, ResourceMonitor, RuntimeInfo } from "./api";
import { nodesForLifecycle } from "./lifecycle";
import { errorMessage, samePort, generatedNodeName, upsertNode, upsertLink } from "./appUtils";
import { SimulatorWorkspace } from "./SimulatorWorkspace";
import type { ConsoleTarget } from "./ConsoleDock";
import { ApplianceManagerPage } from "./ApplianceManagerPage";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { GlobalToast } from "./GlobalToast";
import { NodeDialog } from "./NodeDialog";
import { useAppNodeActions } from "./useAppNodeActions";
import { ProjectsPage } from "./AppProjectsIndex";
import { useProjectSession } from "./useProjectSession";
import { TooltipProvider } from "@/components/ui/tooltip";

type Selection = { kind: "node" | "link"; id: string } | null;
const MarketplaceDialog = lazy(() => import("./MarketplaceDialog").then((module) => ({ default: module.MarketplaceDialog })));
const ConsoleDock = lazy(() => import("./ConsoleDock").then((module) => ({ default: module.ConsoleDock })));
type AppProps = {
  projectId: string | null;
  navigateToProject: (id: string | null) => void;
  showApplianceManager?: boolean;
  navigateToApplianceManager?: () => void;
  onOpenPreferences?: () => void;
  onBackFromApplianceManager?: () => void;
}

function App({ projectId, navigateToProject, showApplianceManager = false, navigateToApplianceManager, onOpenPreferences, onBackFromApplianceManager }: AppProps) {
  const [runtimeInfo, setRuntimeInfo] = useState<RuntimeInfo | null>(null);
  const [resources, setResources] = useState<ResourceMonitor | null>(null);
  const [resourceError, setResourceError] = useState("");
  const [refreshingResources, setRefreshingResources] = useState(false);
  const [filter, setFilter] = useState("");
  const [projectName, setProjectName] = useState("");
  const [creatingProject, setCreatingProject] = useState(false);
  const [creatingNode, setCreatingNode] = useState(false);
  const [nodeDialogOpen, setNodeDialogOpen] = useState(false);
  const [loadingTemplates, setLoadingTemplates] = useState(false);
  const [templateId, setTemplateId] = useState("");
  const [nodeCommand, setNodeCommand] = useState("");
  const [nodeStartupScript, setNodeStartupScript] = useState("");
  const [nodeStartupConfig, setNodeStartupConfig] = useState("");
  const [nodeIdlePc, setNodeIdlePc] = useState("");
  const [uploadingImage, setUploadingImage] = useState(false);
  const [uploadingQemuImage, setUploadingQemuImage] = useState(false);
  const [nodeName, setNodeName] = useState("");
  const [selection, setSelection] = useState<Selection>(null);
  const [connecting, setConnecting] = useState(false);
  const [linkEndpoints, setLinkEndpoints] = useState<LinkEndpoint[]>([]);
  const [startingAll, setStartingAll] = useState(false);
  const [notice, setNotice] = useState("");
  const { projects, setProjects, project, setProject, topology, setTopology, busy, connection, error, setError, templates, setTemplates } = useProjectSession(projectId, setNotice);
  const [consoleTargets, setConsoleTargets] = useState<ConsoleTarget[]>([]);
  const [activeConsoleId, setActiveConsoleId] = useState<string | null>(null);
  const [consoleMinimized, setConsoleMinimized] = useState(true);
  const [marketplaceOpen, setMarketplaceOpen] = useState(false);
  const pendingNodeNames = useRef(new Set<string>());

  useEffect(() => {
    if (!error && !notice) return;
    const timer = window.setTimeout(() => {
      setError("");
      setNotice("");
    }, 6000);
    return () => window.clearTimeout(timer);
  }, [error, notice]);

  useEffect(() => {
    setConsoleTargets([]);
    setActiveConsoleId(null);
    setConsoleMinimized(true);
    setResources(null);
  }, [projectId]);

  const refreshResourceMonitor = useCallback(async () => {
    setRefreshingResources(true);
    try {
      const snapshot = await api.resourceMonitor();
      setResources(snapshot);
      setResourceError("");
    } catch (reason) {
      setResourceError(errorMessage(reason));
    } finally {
      setRefreshingResources(false);
    }
  }, []);

  useEffect(() => {
    void refreshResourceMonitor();
    const timer = window.setInterval(() => void refreshResourceMonitor(), 5000);
    return () => window.clearInterval(timer);
  }, [refreshResourceMonitor]);

  useEffect(() => {
    let active = true;
    api.runtimes()
      .then((status) => active && setRuntimeInfo(status))
      .catch(() => active && setRuntimeInfo({
        docker: { available: false, version: null, error: "Runtime status is unavailable" },
        qemu: { available: false, version: null, qemu_img_available: false, image_directory: "", image_count: 0, acceleration: "TCG", kvm_accessible: false, acceleration_error: "Runtime status is unavailable", error: "Runtime status is unavailable" },
        vpcs: { available: false, version: null, error: "Runtime status is unavailable" },
        dynamips: { available: false, version: null, binary: "dynamips", image_directory: "images/dynamips", image_count: 0, error: "Runtime status is unavailable" },
        ubridge: { available: false, binary: "ubridge", version: null, error: "Runtime status is unavailable" },
      }));
    return () => { active = false; };
  }, []);



  const filteredProjects = useMemo(() => {
    const query = filter.trim().toLowerCase();
    return projects.filter((item) => !query || item.name.toLowerCase().includes(query));
  }, [filter, projects]);

  async function createProject(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const name = projectName.trim();
    if (!name) return;
    try {
      setCreatingProject(true);
      setError("");
      const created = await api.createProject(name);
      setProjectName("");
      navigateToProject(created.project_id);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      setCreatingProject(false);
    }
  }

  async function closeProject() {
    if (!projectId) return;
    try {
      await api.closeProject(projectId);
      navigateToProject(null);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  async function deleteProject(item: Project) {
    if (!window.confirm(`Delete “${item.name}” and its topology?`)) return;
    try {
      await api.deleteProject(item.project_id);
      setProjects((current) => current.filter((candidate) => candidate.project_id !== item.project_id));
      setNotice(`Deleted ${item.name}`);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  const { addNode, openNodeDialog, importDynamipsImage, importQemuImage } = useAppNodeActions({ projectId, topology, nodeName, templateId, nodeCommand, nodeStartupScript, nodeStartupConfig, nodeIdlePc, setCreatingNode, setTopology, setSelection, setNodeName, setNodeDialogOpen, setNotice, setError, setLoadingTemplates, setTemplates, setTemplateId, setNodeCommand, setNodeStartupScript, setNodeStartupConfig, setNodeIdlePc, setUploadingImage, setUploadingQemuImage });

  async function changeNodeLifecycle(nodeId: string, action: "start" | "stop" | "suspend" | "reload") {
    if (!projectId) return;
    try {
      const node = await api[`${action}Node`](projectId, nodeId);
      setTopology((current) => current ? {
        ...current,
        nodes: current.nodes.map((item) => item.node_id === nodeId ? node : item),
      } : current);
      setNotice(`${node.name} ${node.status === "started" ? "is running" : node.status}`);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  async function changeAllNodes(action: "start" | "stop") {
    if (!projectId || !topology || startingAll) return;
    const targets = nodesForLifecycle(topology.nodes, action);
    if (!targets.length) return;
    setStartingAll(true);
    const results = await Promise.allSettled(targets.map((node) => api[`${action}Node`](projectId, node.node_id)));
    const updates = new Map<string, Node>();
    const failures: string[] = [];
    results.forEach((result, index) => {
      if (result.status === "fulfilled") updates.set(targets[index].node_id, result.value);
      else failures.push(`${targets[index].name}: ${errorMessage(result.reason)}`);
    });
    setTopology((current) => current ? { ...current, nodes: current.nodes.map((node) => updates.get(node.node_id) ?? node) } : current);
    setStartingAll(false);
    if (failures.length) setError(`${failures.length} device${failures.length === 1 ? "" : "s"} could not ${action}: ${failures.join("; ")}`);
    else setNotice(`${action === "start" ? "Started" : "Stopped"} ${updates.size} device${updates.size === 1 ? "" : "s"}`);
  }

  async function addTemplateAt(templateId: string, x?: number, y?: number) {
    if (!projectId || !topology) return;
    const template = templates.find((item) => item.template_id === templateId);
    if (!template) {
      setError("That device template is no longer available. Refresh the device library and try again.");
      return;
    }
    const index = topology.nodes.length;
    const position = { x: x ?? -420 + (index % 5) * 180, y: y ?? -220 + Math.floor(index / 5) * 150 };
    const name = template.template_type === "dynamips" ? generatedNodeName(template, topology.nodes, pendingNodeNames.current).replace(/[^a-zA-Z0-9._-]/g, "") : generatedNodeName(template, topology.nodes, pendingNodeNames.current);
    pendingNodeNames.current.add(name);
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
      pendingNodeNames.current.delete(name);
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
      while (usedNames.has(name) || pendingNodeNames.current.has(name)) name = `${baseName} ${suffix++}`;
      reservedName = name;
      payload.name = name;
      payload.x = Number(source.x ?? 0) + 140;
      payload.y = Number(source.y ?? 0) + 100;
      pendingNodeNames.current.add(name);
      const node = await api.createNode(projectId, payload as Omit<Node, "node_id">);
      setTopology((current) => current ? { ...current, nodes: upsertNode(current.nodes, node) } : current);
      setSelection({ kind: "node", id: node.node_id });
      setNotice(`${node.name} duplicated`);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      if (reservedName) pendingNodeNames.current.delete(reservedName);
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

  function openConsole(node: Node, auxiliary = false) {
    if (!projectId || !["docker", "qemu", "vpcs", "dynamips"].includes(node.node_type ?? "")) return;
    const consoleType = String(node.console_type ?? "telnet");
    const properties = node.properties && typeof node.properties === "object" ? node.properties as Record<string, unknown> : {};
    const auxType = String(node.aux_type ?? properties.aux_type ?? "none");
    if (auxiliary && (node.node_type !== "docker" || auxType === "none")) {
      setNotice("Configure a Docker auxiliary console before opening it.");
      return;
    }
    if (!auxiliary && consoleType === "vnc") {
      setNotice("The controller starts VNC on its TCP console port, but this UI has no browser VNC route.");
      return;
    }
    if (!auxiliary && consoleType === "none") {
      setNotice("This device has no console configured.");
      return;
    }
    if (!auxiliary && node.node_type === "docker" && (consoleType === "http" || consoleType === "https")) {
      const host = typeof node.console_host === "string" && node.console_host ? node.console_host : window.location.hostname;
      const url = dockerHttpConsoleUrl(consoleType, host, Number(node.console_http_port ?? 80), String(node.console_http_path ?? "/"));
      if (!url) {
        setNotice("The HTTP console address is invalid. Check its host, port, and path in device settings.");
        return;
      }
      window.open(url, "_blank", "noopener,noreferrer");
      setNotice("Opened the configured HTTP console. The container port must be reachable from your browser.");
      return;
    }
    const targetId = `${node.node_id}:${auxiliary ? "aux" : "main"}`;
    const target: ConsoleTarget = { id: targetId, projectId, nodeId: node.node_id, nodeName: node.name, nodeType: node.node_type as ConsoleTarget["nodeType"], auxiliary };
    setConsoleTargets((current) => current.some((item) => item.id === targetId) ? current : [...current, target]);
    setActiveConsoleId(targetId);
    setConsoleMinimized(false);
  }

  function selectPort(endpoint: LinkEndpoint) {
    if (!connecting) return;
    setLinkEndpoints((current) => current.some((item) => samePort(item, endpoint))
      ? current.filter((item) => !samePort(item, endpoint))
      : [...current, endpoint]);
    setSelection({ kind: "node", id: endpoint.node_id });
  }

  async function createSelectedLink() {
    if (!projectId || !topology || linkEndpoints.length < 2) return;
    try {
      const link = await api.createLink(projectId, linkEndpoints);
      setTopology((current) => current ? { ...current, links: upsertLink(current.links, link) } : current);
      setSelection({ kind: "link", id: link.link_id });
      setLinkEndpoints([]);
      setConnecting(false);
      setNotice(`Ethernet segment created with ${link.nodes.length} endpoints`);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  async function saveNodePosition(nodeId: string, x: number, y: number) {
    if (!projectId) return;
    try {
      const node = await api.updateNode(projectId, nodeId, { x, y });
      setTopology((current) => current ? {
        ...current,
        nodes: current.nodes.map((item) => item.node_id === nodeId ? node : item),
      } : current);
    } catch (reason) {
      setError(errorMessage(reason));
      void reloadTopology();
    }
  }

  async function deleteSelection() {
    if (!projectId || !selection) return;
    try {
      if (selection.kind === "node") await api.deleteNode(projectId, selection.id);
      else await api.deleteLink(projectId, selection.id);
      setSelection(null);
      setTopology((current) => {
        if (!current) return current;
        if (selection.kind === "link") {
          return { ...current, links: current.links.filter((link) => link.link_id !== selection.id) };
        }
        return {
          ...current,
          nodes: current.nodes.filter((node) => node.node_id !== selection.id),
          links: current.links.filter((link) => !link.nodes.some((endpoint) => endpoint.node_id === selection.id)),
        };
      });
      setNotice("Selection removed");
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  async function reloadTopology() {
    if (!projectId) return;
    try {
      const current = await api.topology(projectId);
      setProject(current.project);
      setTopology(current);
    } catch (reason) {
      setError(errorMessage(reason));
    }
  }

  function updateLocalPosition(nodeId: string, x: number, y: number) {
    setTopology((current) => current ? {
      ...current,
      nodes: current.nodes.map((node) => node.node_id === nodeId ? { ...node, x, y } : node),
    } : current);
  }

  const selectedNode = selection?.kind === "node"
    ? topology?.nodes.find((node) => node.node_id === selection.id)
    : null;
  const selectedLink = selection?.kind === "link"
    ? topology?.links.find((link) => link.link_id === selection.id)
    : null;

  return (
    <TooltipProvider>
      <div className="app-shell">
        {!projectId && !showApplianceManager && <header className="sim-app-header">
          <Button variant="ghost" className="sim-brand" onClick={() => navigateToProject(null)}>
            <span className="sim-brand-mark"><Network size={18} /></span>
            <span>Network Simulator</span>
          </Button>
          <div className="sim-header-actions">
            <Badge variant={connection === "connected" ? "secondary" : connection === "disconnected" ? "destructive" : "outline"} className="gap-1.5">
              <span className={`sim-connection-dot ${connection}`} />{connection === "connected" ? "Controller connected" : connection === "reconnecting" ? "Reconnecting" : connection === "checking" ? "Connecting" : "Controller unavailable"}
            </Badge>
          </div>
        </header>}

        <GlobalToast error={error} notice={notice} onDismiss={() => { setError(""); setNotice(""); }} />

        <main className="main-area">
          {showApplianceManager ? <ApplianceManagerPage onBack={onBackFromApplianceManager ?? (() => navigateToProject(projectId))} onOpenPreferences={onOpenPreferences} onOpenImageCatalog={() => setMarketplaceOpen(true)} onTemplatesChanged={setTemplates} /> : projectId ? (busy || !project || !topology ? <div className="sim-loading"><Activity className="animate-pulse" />Loading project topology…</div> : <SimulatorWorkspace
            project={project}
            topology={topology}
            templates={templates}
            runtimeInfo={runtimeInfo}
            resources={resources}
            resourceError={resourceError}
            refreshingResources={refreshingResources}
            connection={connection}
            batchLifecycleRunning={startingAll}
            selection={selection}
            selectedNode={selectedNode}
            selectedLink={selectedLink}
            connecting={connecting}
            linkEndpoints={linkEndpoints}
            onBack={closeProject}
            onRefreshResources={() => void refreshResourceMonitor()}
            onOpenNodeDialog={() => void openNodeDialog()}
            onOpenMarketplace={() => setMarketplaceOpen(true)}
            onOpenApplianceManager={navigateToApplianceManager ?? (() => setMarketplaceOpen(true))}
            onOpenPreferences={onOpenPreferences ?? (() => undefined)}
            onAddTemplate={(id, x, y) => void addTemplateAt(id, x, y)}
            onNodeLifecycle={changeNodeLifecycle}
            onStartAll={() => void changeAllNodes("start")}
            onStopAll={() => void changeAllNodes("stop")}
            onOpenConsole={openConsole}
            onToggleConnect={() => { setConnecting((value) => !value); setLinkEndpoints([]); setNotice(connecting ? "Port connection mode cancelled" : "Select interface ports to connect"); }}
            onCreateLink={() => void createSelectedLink()}
            onDeleteSelection={() => void deleteSelection()}
            onNodeRename={renameNode}
            onDuplicateNode={duplicateNode}
            onConfigureNode={configureNode}
            onSaveTemplate={saveTemplateSettings}
            onSelectNode={(id) => setSelection({ kind: "node", id })}
            onSelectLink={(id) => setSelection({ kind: "link", id })}
            onSelectCanvas={() => setSelection(null)}
            onSelectPort={selectPort}
            onNodeContext={(id) => setSelection({ kind: "node", id })}
            onNodeMove={updateLocalPosition}
            onNodeMoveEnd={saveNodePosition}
          />) : <ProjectsPage projects={projects} filteredProjects={filteredProjects} busy={busy} filter={filter} projectName={projectName} creatingProject={creatingProject} onFilterChange={setFilter} onProjectNameChange={setProjectName} onCreateProject={createProject} onNavigate={navigateToProject} onOpenAppliances={navigateToApplianceManager} onOpenMarketplace={() => setMarketplaceOpen(true)} onDeleteProject={(item) => void deleteProject(item)} />}
        </main>

        {projectId && consoleTargets.length > 0 && <Suspense fallback={<div className="sim-console-loading">Loading console…</div>}><ConsoleDock
          targets={consoleTargets}
          activeNodeId={activeConsoleId}
          minimized={consoleMinimized}
          onSelect={setActiveConsoleId}
          onMinimize={() => setConsoleMinimized(true)}
          onRestore={() => setConsoleMinimized(false)}
          onClose={(targetId) => { setConsoleTargets((current) => current.filter((target) => target.id !== targetId)); setActiveConsoleId((current) => current === targetId ? (consoleTargets.find((target) => target.id !== targetId)?.id ?? null) : current); }}
        /></Suspense>}

      <NodeDialog open={nodeDialogOpen} onOpenChange={setNodeDialogOpen} onSubmit={addNode} nodeName={nodeName} setNodeName={setNodeName} templates={templates} templateId={templateId} setTemplateId={setTemplateId} setNodeCommand={setNodeCommand} setNodeStartupScript={setNodeStartupScript} setNodeStartupConfig={setNodeStartupConfig} setNodeIdlePc={setNodeIdlePc} nodeCommand={nodeCommand} nodeStartupScript={nodeStartupScript} nodeStartupConfig={nodeStartupConfig} nodeIdlePc={nodeIdlePc} runtimeInfo={runtimeInfo} loadingTemplates={loadingTemplates} creatingNode={creatingNode} uploadingImage={uploadingImage} uploadingQemuImage={uploadingQemuImage} onImportImage={(file) => void importDynamipsImage(file)} onImportQemuImage={(file) => void importQemuImage(file)} />

      {marketplaceOpen && <Suspense fallback={<div className="sim-dialog-loading">Loading image catalog…</div>}><MarketplaceDialog onClose={() => setMarketplaceOpen(false)} /></Suspense>}
      </div>
    </TooltipProvider>
  );
}

export default App;
