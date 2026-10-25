import { lazy, Suspense, useCallback, useEffect, useRef, useState } from "react";
import { Activity } from "lucide-react";
import { api, dockerHttpConsoleUrl } from "./api";
import type { LinkEndpoint, Node, ResourceMonitor, RuntimeInfo } from "./api";
import { nodesForLifecycle } from "./lifecycle";
import { errorMessage, samePort, upsertLink } from "./appUtils";
import { SimulatorWorkspace } from "./SimulatorWorkspace";
import type { ConsoleTarget } from "./ConsoleDock";
import { ApplianceManagerPage } from "./ApplianceManagerPage";
import { PagesShell } from "./PagesShell";
import { GlobalToast } from "./GlobalToast";
import { NodeDialog } from "./NodeDialog";
import { AppNavigationHeader } from "./AppNavigationHeader";
import { useAppNodeActions } from "./useAppNodeActions";
import { useAppDeviceActions } from "./useAppDeviceActions";
import { ProjectsPage } from "./ProjectsPage";
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
  const { project, setProject, topology, setTopology, busy, connection, error, setError, templates, setTemplates } = useProjectSession(projectId, setNotice);
  const [consoleTargets, setConsoleTargets] = useState<ConsoleTarget[]>([]);
  const [activeConsoleId, setActiveConsoleId] = useState<string | null>(null);
  const [consoleMinimized, setConsoleMinimized] = useState(() => { try { return JSON.parse(localStorage.getItem("ns.console.float") ?? "{}").minimized ?? true; } catch { return true; } });
  const [consoleDockOpen, setConsoleDockOpen] = useState(() => { try { return JSON.parse(localStorage.getItem("ns.console.float") ?? "{}").open ?? false; } catch { return false; } });
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



  async function closeProject() {
    if (!projectId) return;
    try {
      await api.closeProject(projectId);
      navigateToProject(null);
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

  const { addTemplateAt, renameNode, duplicateNode, configureNode, saveTemplateSettings } = useAppDeviceActions({ projectId, topology, templates, setError, setNotice, setTopology, setSelection, setConsoleTargets, pendingNodeNames: pendingNodeNames.current, setTemplates });

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
    setConsoleDockOpen(true);
    setConsoleMinimized(false);
  }

  function selectPort(endpoint: LinkEndpoint) {
    if (!connecting) return;
    const first = linkEndpoints[0];
    if (first && !samePort(first, endpoint)) {
      if (!projectId) return;
      void api.createLink(projectId, [first, endpoint]).then((link) => {
        setTopology((current) => current ? { ...current, links: upsertLink(current.links, link) } : current);
        setSelection({ kind: "link", id: link.link_id });
        setLinkEndpoints([]);
        setConnecting(false);
        setNotice(`Ethernet segment created with ${link.nodes.length} endpoints`);
      }).catch((reason) => setError(errorMessage(reason)));
    } else if (first && samePort(first, endpoint)) {
      setLinkEndpoints([]);
    } else setLinkEndpoints([endpoint]);
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

  async function saveNodePosition(positions: Array<{ id: string; x: number; y: number }>) {
    if (!projectId) return;
    try {
      for (const position of positions) await api.updateNode(projectId, position.id, { x: position.x, y: position.y });
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
        {!projectId && !showApplianceManager && <AppNavigationHeader connection={connection} onNavigateHome={() => navigateToProject(null)} />}

        <GlobalToast error={error} notice={notice} onDismiss={() => { setError(""); setNotice(""); }} />

        <main className="main-area">
          {showApplianceManager ? <PagesShell active="Appliances"><ApplianceManagerPage onBack={onBackFromApplianceManager ?? (() => navigateToProject(projectId))} onOpenPreferences={onOpenPreferences} onOpenImageCatalog={() => setMarketplaceOpen(true)} onTemplatesChanged={setTemplates} /></PagesShell> : projectId ? (busy || !project || !topology ? <div className="sim-loading"><Activity className="animate-pulse" />Loading project topology…</div> : <SimulatorWorkspace
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
            consoleOpen={consoleDockOpen}
            onCloseConsoleDock={() => { setConsoleDockOpen(false); try { const v=JSON.parse(localStorage.getItem("ns.console.float")??"{}"); localStorage.setItem("ns.console.float",JSON.stringify({...v,open:false})); } catch {} }}
            onOpenConsoleDock={() => { setConsoleDockOpen(true); setConsoleMinimized(false); }}
            onToggleConsoleDock={() => { if (consoleDockOpen) setConsoleDockOpen(false); else { setConsoleDockOpen(true); setConsoleMinimized(false); } }}
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
          />) : <ProjectsPage onNavigateProject={(id) => navigateToProject(id)} onOpenAppliances={navigateToApplianceManager} />}
        </main>

        {projectId && consoleDockOpen && <Suspense fallback={<div className="sim-console-loading">Loading console…</div>}><ConsoleDock
          targets={consoleTargets}
          activeNodeId={activeConsoleId}
          minimized={consoleMinimized}
          onSelect={setActiveConsoleId}
          onMinimize={() => setConsoleMinimized(true)}
          onRestore={() => setConsoleMinimized(false)}
          onHide={() => { setConsoleDockOpen(false); try { const v=JSON.parse(localStorage.getItem("ns.console.float")??"{}"); localStorage.setItem("ns.console.float",JSON.stringify({...v,open:false})); } catch {} }}
          onClose={(targetId) => { setConsoleTargets((current) => current.filter((target) => target.id !== targetId)); setActiveConsoleId((current) => current === targetId ? (consoleTargets.find((target) => target.id !== targetId)?.id ?? null) : current); }}
        /></Suspense>}

      <NodeDialog open={nodeDialogOpen} onOpenChange={setNodeDialogOpen} onSubmit={addNode} nodeName={nodeName} setNodeName={setNodeName} templates={templates} templateId={templateId} setTemplateId={setTemplateId} setNodeCommand={setNodeCommand} setNodeStartupScript={setNodeStartupScript} setNodeStartupConfig={setNodeStartupConfig} setNodeIdlePc={setNodeIdlePc} nodeCommand={nodeCommand} nodeStartupScript={nodeStartupScript} nodeStartupConfig={nodeStartupConfig} nodeIdlePc={nodeIdlePc} runtimeInfo={runtimeInfo} loadingTemplates={loadingTemplates} creatingNode={creatingNode} uploadingImage={uploadingImage} uploadingQemuImage={uploadingQemuImage} onImportImage={(file) => void importDynamipsImage(file)} onImportQemuImage={(file) => void importQemuImage(file)} />

      {marketplaceOpen && <Suspense fallback={<div className="sim-dialog-loading">Loading image catalog…</div>}><MarketplaceDialog onClose={() => setMarketplaceOpen(false)} /></Suspense>}
      </div>
    </TooltipProvider>
  );
}

export default App;
