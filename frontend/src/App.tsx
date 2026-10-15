import { useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent, PointerEvent } from "react";
import { api, projectNotificationsUrl } from "./api";
import type { Drawing, Link, Node, Project, ProjectEvent, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { applyProjectEvent, sanitizeDrawingSvg } from "./topology";

type Selection = { kind: "node" | "link"; id: string } | null;

const scene = { x: -620, y: -380, width: 1240, height: 760 };

type AppProps = {
  projectId: string | null;
  navigateToProject: (id: string | null) => void;
}

function App({ projectId, navigateToProject }: AppProps) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [runtimeInfo, setRuntimeInfo] = useState<RuntimeInfo | null>(null);
  const [project, setProject] = useState<Project | null>(null);
  const [topology, setTopology] = useState<TopologySnapshot | null>(null);
  const [busy, setBusy] = useState(true);
  const [connection, setConnection] = useState<"checking" | "connected" | "reconnecting" | "disconnected">("checking");
  const [error, setError] = useState("");
  const [filter, setFilter] = useState("");
  const [projectName, setProjectName] = useState("");
  const [creatingProject, setCreatingProject] = useState(false);
  const [creatingNode, setCreatingNode] = useState(false);
  const [nodeDialogOpen, setNodeDialogOpen] = useState(false);
  const [loadingTemplates, setLoadingTemplates] = useState(false);
  const [templates, setTemplates] = useState<RuntimeTemplate[]>([]);
  const [templateId, setTemplateId] = useState("");
  const [nodeCommand, setNodeCommand] = useState("");
  const [nodeName, setNodeName] = useState("");
  const [selection, setSelection] = useState<Selection>(null);
  const [connecting, setConnecting] = useState(false);
  const [firstEndpoint, setFirstEndpoint] = useState<string | null>(null);
  const [notice, setNotice] = useState("");
  const socketRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    let active = true;
    api.runtimes()
      .then((status) => active && setRuntimeInfo(status))
      .catch(() => active && setRuntimeInfo({
        docker: { available: false, version: null, error: "Runtime status is unavailable" },
        qemu: { available: false, version: null, qemu_img_available: false, image_directory: "", image_count: 0, acceleration: "TCG", error: "Runtime status is unavailable" },
      }));
    return () => { active = false; };
  }, []);

  useEffect(() => {
    let active = true;
    setBusy(true);
    setConnection("checking");
    setError("");
    setProject(null);
    setTopology(null);

    if (!projectId) {
      api.listProjects()
        .then((items) => {
          if (active) {
            setProjects(items);
            setConnection("connected");
          }
        })
        .catch((reason: unknown) => {
          if (active) {
            setConnection("disconnected");
            setError(errorMessage(reason));
          }
        })
        .finally(() => active && setBusy(false));
      return () => { active = false; };
    }

    let reconnectTimer = 0;
    let cursor = 0;
    const connect = () => {
      if (!active) return;
      const socket = new WebSocket(projectNotificationsUrl(projectId, cursor));
      socketRef.current = socket;
      socket.onmessage = (message) => {
        try {
          const event = JSON.parse(message.data) as ProjectEvent;
          cursor = Math.max(cursor, event.sequence);
          setTopology((current) => current ? applyProjectEvent(current, event) : current);
        } catch {
          setNotice("A project update could not be read. Refresh to reload the topology.");
        }
      };
      socket.onopen = () => setConnection("connected");
      socket.onclose = () => {
        if (active) {
          setConnection("reconnecting");
          reconnectTimer = window.setTimeout(connect, 1200);
        }
      };
      socket.onerror = () => socket.close();
    };

    const loadProject = async () => {
      try {
        await api.openProject(projectId);
        const snapshot = await api.topology(projectId);
        if (!active) return;
        cursor = snapshot.event_cursor;
        setProject(snapshot.project);
        setTopology(snapshot);
        setConnection("connected");
        connect();
      } catch (reason) {
        if (active) {
          setConnection("disconnected");
          setError(errorMessage(reason));
        }
      } finally {
        if (active) setBusy(false);
      }
    };
    void loadProject();

    return () => {
      active = false;
      window.clearTimeout(reconnectTimer);
      socketRef.current?.close();
      socketRef.current = null;
    };
  }, [projectId]);

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
      });
      setTopology((current) => current ? { ...current, nodes: [...current.nodes, node] } : current);
      setSelection({ kind: "node", id: node.node_id });
      setNodeName("");
      setNodeDialogOpen(false);
      setNotice(`${node.name} ${node.node_type === "qemu" ? "virtual machine" : "container"} created and added to the topology`);
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
    } catch (reason) {
      setTemplates([]);
      setTemplateId("");
      setError(errorMessage(reason));
    } finally {
      setLoadingTemplates(false);
    }
  }

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

  async function connectNodes(nodeId: string) {
    if (!projectId || !topology) return;
    if (!firstEndpoint) {
      setFirstEndpoint(nodeId);
      setNotice("Choose a second node to connect");
      return;
    }
    if (firstEndpoint === nodeId) return;
    const usedPorts = new Map<string, Set<number>>();
    for (const link of topology.links) {
      for (const endpoint of link.nodes ?? []) {
        if (!usedPorts.has(endpoint.node_id)) usedPorts.set(endpoint.node_id, new Set());
        const node = topology.nodes.find((item) => item.node_id === endpoint.node_id);
        usedPorts.get(endpoint.node_id)?.add(node?.node_type === "qemu" ? endpoint.adapter_number : endpoint.port_number);
      }
    }
    const nextFreePort = (id: string) => {
      const ports = usedPorts.get(id) ?? new Set<number>();
      let port = 0;
      while (ports.has(port)) port += 1;
      return port;
    };
    const endpointFor = (id: string) => {
      const node = topology.nodes.find((item) => item.node_id === id);
      const next = nextFreePort(id);
      return node?.node_type === "qemu"
        ? { node_id: id, adapter_number: next, port_number: 0 }
        : { node_id: id, adapter_number: 0, port_number: next };
    };
    try {
      const link = await api.createLink(projectId, [firstEndpoint, nodeId].map(endpointFor));
      setTopology((current) => current ? { ...current, links: [...current.links, link] } : current);
      setSelection({ kind: "link", id: link.link_id });
      setFirstEndpoint(null);
      setConnecting(false);
      setNotice("Link created");
    } catch (reason) {
      setError(errorMessage(reason));
      setFirstEndpoint(null);
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
    <div className="app-shell">
      <aside className="sidebar">
        <a className="brand" href="/" onClick={(event) => { event.preventDefault(); navigateToProject(null); }}>
          <span className="brand-mark">N</span>
          <span><b>network</b><small>STUDIO</small></span>
        </a>
        <div className="sidebar-label">WORKSPACE</div>
        <button className={`nav-item ${!projectId ? "active" : ""}`} onClick={() => navigateToProject(null)}>
          <span className="nav-icon">▦</span> Projects
        </button>
        <div className="sidebar-label recent-label">RECENT LABS</div>
        {projects.slice(0, 4).map((item) => (
          <button className="recent-item" key={item.project_id} onClick={() => navigateToProject(item.project_id)}>
            <span className="recent-dot" />{item.name}
          </button>
        ))}
        <div className="sidebar-spacer" />
        <div className="server-card">
          <span className={`server-light ${runtimeInfo?.docker.available ? "" : "runtime-offline"}`} />
          <div><b>Docker Engine</b><small>{runtimeInfo?.docker.available ? `Available · ${runtimeInfo.docker.version}` : runtimeInfo ? "Unavailable" : "Checking…"}</small></div>
          <span className="server-menu">•••</span>
        </div>
        <div className="server-card">
          <span className={`server-light ${runtimeInfo?.qemu.available ? "" : "runtime-offline"}`} />
          <div><b>QEMU / KVM</b><small>{runtimeInfo?.qemu.available ? `Available · ${runtimeInfo.qemu.acceleration}` : runtimeInfo ? "Unavailable" : "Checking…"}</small></div>
          <span className="server-menu">•••</span>
        </div>
        <div className="sidebar-footer">Network Studio <span>0.1.0</span></div>
      </aside>

      <main className="main-area">
        <header className="topbar">
          <div className="breadcrumbs"><span>Workspace</span><i>/</i><b>{project?.name ?? (projectId ? "Project" : "Projects")}</b></div>
          <div className="topbar-actions">
            <span className={`connection-pill ${connection}`}><span className="status-light" />{connection === "connected" ? "API connected" : connection === "reconnecting" ? "Reconnecting…" : connection === "checking" ? "Checking API…" : "API unavailable"}</span>
            <button className="avatar" aria-label="User menu">T</button>
          </div>
        </header>

        {error && <div className="toast toast-error" role="alert"><span>{error}</span><button onClick={() => setError("")} aria-label="Dismiss error">×</button></div>}
        {notice && <div className="toast toast-success" role="status"><span>{notice}</span><button onClick={() => setNotice("")} aria-label="Dismiss message">×</button></div>}

        {projectId ? (
          <ProjectWorkspace
            busy={busy}
            project={project}
            topology={topology}
            selection={selection}
            selectedNode={selectedNode}
            selectedLink={selectedLink}
            connecting={connecting}
            firstEndpoint={firstEndpoint}
            onBack={closeProject}
            onOpenNodeDialog={() => void openNodeDialog()}
            onNodeLifecycle={changeNodeLifecycle}
            onToggleConnect={() => { setConnecting((value) => !value); setFirstEndpoint(null); setNotice("Select two nodes to add a link"); }}
            onDeleteSelection={deleteSelection}
            onNodeClick={(id) => connecting ? void connectNodes(id) : setSelection({ kind: "node", id })}
            onLinkClick={(id) => setSelection({ kind: "link", id })}
            onNodeMove={updateLocalPosition}
            onNodeMoveEnd={saveNodePosition}
          />
        ) : (
          <section className="projects-page">
            <div className="page-heading">
              <div>
                <div className="eyebrow">YOUR WORKSPACE</div>
                <h1>Projects</h1>
                <p>Build, connect, and manage your network labs.</p>
              </div>
              <button className="primary-button" onClick={() => document.getElementById("new-project-name")?.focus()}>
                <span>＋</span> New project
              </button>
            </div>

            <form className="create-project-card" onSubmit={createProject}>
              <div className="create-icon">＋</div>
              <div className="create-copy"><b>Start a new lab</b><span>Give your project a name to create a blank topology.</span></div>
              <div className="create-fields">
                <input id="new-project-name" aria-label="New project name" placeholder="e.g. Branch office network" value={projectName} onChange={(event) => setProjectName(event.target.value)} />
                <button className="primary-button" type="submit" disabled={!projectName.trim() || creatingProject}>
                  {creatingProject ? "Creating…" : "Create project"}
                </button>
              </div>
            </form>

            <div className="section-heading">
              <div><h2>All projects <span className="count-badge">{projects.length}</span></h2><p>Open a project to continue working on its topology.</p></div>
              <label className="search-box"><span>⌕</span><input aria-label="Filter projects" placeholder="Find a project" value={filter} onChange={(event) => setFilter(event.target.value)} /></label>
            </div>

            {busy ? <div className="loading-card">Loading projects…</div> : filteredProjects.length ? (
              <div className="project-grid">
                {filteredProjects.map((item) => (
                  <article className="project-card" key={item.project_id}>
                    <div className="project-card-top"><div className="project-glyph">⌘</div><button className="icon-button" aria-label={`Delete ${item.name}`} onClick={() => void deleteProject(item)}>•••</button></div>
                    <h3>{item.name}</h3>
                    <div className="project-meta"><span className="project-status"><i />{item.status ?? "closed"}</span><span>Local project</span></div>
                    <button className="card-open" onClick={() => navigateToProject(item.project_id)}>Open project <span>→</span></button>
                  </article>
                ))}
              </div>
            ) : projects.length ? (
              <div className="empty-card"><div className="empty-icon">⌕</div><b>No matching projects</b><span>Try another search term.</span></div>
            ) : (
              <div className="empty-card"><div className="empty-icon">◇</div><b>No projects yet</b><span>Create a project above to start your first network lab.</span></div>
            )}
          </section>
        )}
      </main>

      {nodeDialogOpen && (
        <div className="modal-backdrop" role="presentation" onMouseDown={(event) => { if (event.target === event.currentTarget) setNodeDialogOpen(false); }}>
          <form className="modal-card" onSubmit={addNode}>
            <div className="modal-heading"><div><div className="eyebrow">TOPOLOGY</div><h2>Add a node</h2></div><button type="button" className="icon-button" onClick={() => setNodeDialogOpen(false)} aria-label="Close">×</button></div>
            <label className="field-label" htmlFor="node-name">Node name</label>
            <input id="node-name" autoFocus placeholder="e.g. Core switch" value={nodeName} onChange={(event) => setNodeName(event.target.value)} />
            <label className="field-label" htmlFor="node-template">Installed runtime image</label>
            {loadingTemplates ? <div className="runtime-empty" role="status">Discovering local runtime images…</div> : templates.length ? (
              <select id="node-template" value={templateId} onChange={(event) => {
                const selected = templates.find((item) => item.template_id === event.target.value);
                setTemplateId(event.target.value);
                setNodeCommand(selected?.template_type === "docker" ? selected.start_command ?? "" : "");
              }}>
                {templates.map((template) => <option key={template.template_id} value={template.template_id}>{template.template_type === "qemu" ? "QEMU · " : "Docker · "}{template.name}</option>)}
              </select>
            ) : (
              <div className="runtime-empty" role="status">
                {runtimeInfo?.docker.available || runtimeInfo?.qemu.available
                  ? <>No local runtime images. Pull a Docker image or place a QEMU disk image in <code>{runtimeInfo.qemu.image_directory || "images/qemu"}</code>.</>
                  : <>Docker Engine and QEMU are unavailable. Install a runtime and make sure this service account can access it.</>}
              </div>
            )}
            {templates.find((item) => item.template_id === templateId)?.template_type === "docker" && <>
              <label className="field-label" htmlFor="node-command">Container command (optional)</label>
              <input id="node-command" placeholder="e.g. /bin/sh -c 'sleep 3600'" value={nodeCommand} onChange={(event) => setNodeCommand(event.target.value)} />
            </>}
            {templates.find((item) => item.template_id === templateId)?.template_type === "qemu" ? (
              <div className="node-type-note"><span className="node-mini-icon">▣</span><span><b>QEMU virtual machine</b><small>Creates a private copy-on-write disk. KVM is used when accessible; otherwise QEMU uses TCG.</small></span></div>
            ) : templates.length > 0 ? (
              <div className="node-type-note"><span className="node-mini-icon">▦</span><span><b>Docker container</b><small>Creates a stopped container from the selected local image.</small></span></div>
            ) : null}
            <div className="modal-actions"><button type="button" className="secondary-button" onClick={() => setNodeDialogOpen(false)}>Cancel</button><button className="primary-button" type="submit" disabled={!nodeName.trim() || !templateId || loadingTemplates || creatingNode}>{creatingNode ? "Creating…" : "Create node"}</button></div>
          </form>
        </div>
      )}
    </div>
  );
}

type WorkspaceProps = {
  busy: boolean;
  project: Project | null;
  topology: TopologySnapshot | null;
  selection: Selection;
  selectedNode: Node | null | undefined;
  selectedLink: Link | null | undefined;
  connecting: boolean;
  firstEndpoint: string | null;
  onBack: () => void;
  onOpenNodeDialog: () => void;
  onNodeLifecycle: (id: string, action: "start" | "stop" | "suspend" | "reload") => void;
  onToggleConnect: () => void;
  onDeleteSelection: () => void;
  onNodeClick: (id: string) => void;
  onLinkClick: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
};

function ProjectWorkspace(props: WorkspaceProps) {
  const [zoom, setZoom] = useState(1);
  const { project, topology, busy } = props;
  if (busy || !project || !topology) return <div className="workspace-loading"><span className="spinner" />Loading project topology…</div>;
  const selected = props.selectedNode ?? props.selectedLink;

  return (
    <section className="workspace-page">
      <div className="workspace-heading">
        <div className="workspace-title"><button className="back-button" onClick={props.onBack} aria-label="Back to projects">←</button><div><div className="eyebrow">NETWORK LAB</div><h1>{project.name}</h1></div><span className="status-tag"><i />{project.status ?? "opened"}</span></div>
        <div className="workspace-tools">
          <button className="secondary-button" onClick={props.onToggleConnect} aria-pressed={props.connecting}><span className="button-glyph">⌁</span>{props.connecting ? "Cancel link" : "Connect nodes"}</button>
          <button className="primary-button" onClick={props.onOpenNodeDialog}><span>＋</span> Add node</button>
          <button className="icon-button delete-button" onClick={props.onDeleteSelection} disabled={!props.selection} aria-label="Delete selected item" title="Delete selected item">⌫</button>
        </div>
      </div>

      {props.connecting && <div className="connect-hint">{props.firstEndpoint ? "Now select the other node to finish the link." : "Select the first node, then the second node."}<button onClick={props.onToggleConnect}>Cancel</button></div>}

      <div className="workspace-layout">
        <div className="canvas-card">
          <div className="canvas-toolbar"><div className="canvas-title"><span className="canvas-live"><i />Live</span><b>Topology</b><span className="canvas-separator" />{topology.nodes.length} nodes <span>·</span> {topology.links.length} links</div><div className="canvas-actions"><button aria-label="Zoom out" title="Zoom out" onClick={() => setZoom((value) => Math.max(0.6, Number((value - 0.1).toFixed(1))))}>−</button><span>{Math.round(zoom * 100)}%</span><button aria-label="Zoom in" title="Zoom in" onClick={() => setZoom((value) => Math.min(1.8, Number((value + 0.1).toFixed(1))))}>＋</button><button aria-label="Fit topology" title="Fit topology" onClick={() => setZoom(1)}>⤢</button></div></div>
          <TopologyCanvas
            nodes={topology.nodes}
            links={topology.links}
            drawings={topology.drawings}
            zoom={zoom}
            selection={props.selection}
            connecting={props.connecting}
            firstEndpoint={props.firstEndpoint}
            onNodeClick={props.onNodeClick}
            onLinkClick={props.onLinkClick}
            onNodeMove={props.onNodeMove}
            onNodeMoveEnd={props.onNodeMoveEnd}
          />
          <div className="canvas-footer"><span><kbd>Drag</kbd> to reposition</span><span><kbd>Click</kbd> to select</span><span>Changes save automatically</span></div>
        </div>

        <aside className="inspector-card">
          <div className="inspector-heading"><div><div className="eyebrow">PROJECT</div><h2>Overview</h2></div><button className="icon-button" aria-label="Project options">•••</button></div>
          <div className="stat-row"><div className="stat-icon node-stat">▦</div><div><b>{topology.nodes.length}</b><span>Nodes</span></div><div className="stat-icon link-stat">⌁</div><div><b>{topology.links.length}</b><span>Links</span></div></div>
          <div className="inspector-divider" />
          <div className="inspector-section"><div className="inspector-section-title">{selected ? "SELECTED ITEM" : "TOPOLOGY OBJECTS"}</div>
            {props.selectedNode ? <>
              <div className="selected-details"><div className="detail-icon">▣</div><div><b>{props.selectedNode.name}</b><span>{typeof props.selectedNode.image === "string" ? props.selectedNode.image : typeof props.selectedNode.hda_disk_image === "string" ? props.selectedNode.hda_disk_image.split("/").at(-1) : props.selectedNode.node_type ?? "Node"}</span><span className="detail-id">{props.selectedNode.node_id.slice(0, 8)}</span></div></div>
              <div className="runtime-controls">
                <span className={`runtime-state ${props.selectedNode.status === "started" ? "running" : ""}`}>{props.selectedNode.status ?? "stopped"}{props.selectedNode.runtime_missing ? " · runtime missing" : ""}</span>
                <div className="runtime-actions">
                  <button className="secondary-button" onClick={() => props.onNodeLifecycle(props.selectedNode!.node_id, props.selectedNode!.status === "started" ? "stop" : "start")} disabled={Boolean(props.selectedNode.runtime_missing)}>{props.selectedNode.status === "started" ? "Stop" : "Start"}</button>
                  <button className="icon-button" onClick={() => props.onNodeLifecycle(props.selectedNode!.node_id, "suspend")} disabled={props.selectedNode.status !== "started"} title="Suspend node" aria-label="Suspend node">Ⅱ</button>
                  <button className="icon-button" onClick={() => props.onNodeLifecycle(props.selectedNode!.node_id, "reload")} disabled={props.selectedNode.status !== "started"} title="Restart node" aria-label="Restart node">↻</button>
                </div>
              </div>
            </> : null}
            {props.selectedLink ? <div className="selected-details"><div className="detail-icon link-detail-icon">⌁</div><div><b>Ethernet link</b><span>{props.selectedLink.nodes.length} endpoints</span><span className="detail-id">{props.selectedLink.link_id.slice(0, 8)}</span></div></div> : null}
            {!selected && <div className="object-list">
              {topology.nodes.slice(0, 8).map((node) => <button key={node.node_id} onClick={() => props.onNodeClick(node.node_id)}><span className="object-glyph">▦</span><span>{node.name}</span><i className={node.status === "started" ? "running-dot" : "stopped-dot"} /></button>)}
              {topology.nodes.length > 8 && <span className="more-items">+{topology.nodes.length - 8} more nodes</span>}
              {topology.nodes.length === 0 && <span className="muted-small">Add a Docker container or QEMU virtual machine to begin.</span>}
            </div>}
          </div>
          <div className="inspector-divider" />
          <div className="inspector-section"><div className="inspector-section-title">PROJECT INFO</div><div className="info-row"><span>Drawings</span><b>{topology.drawings.length}</b></div><div className="info-row"><span>Event cursor</span><b>#{topology.event_cursor}</b></div><div className="info-row"><span>Controller</span><b className="controller-ok">Connected</b></div></div>
          {selected && <button className="danger-action" onClick={props.onDeleteSelection}>Delete selected item</button>}
          <div className="inspector-footnote">Project changes sync to the local controller.</div>
        </aside>
      </div>
    </section>
  );
}

type CanvasProps = {
  nodes: Node[];
  links: Link[];
  drawings: Drawing[];
  zoom: number;
  selection: Selection;
  connecting: boolean;
  firstEndpoint: string | null;
  onNodeClick: (id: string) => void;
  onLinkClick: (id: string) => void;
  onNodeMove: (id: string, x: number, y: number) => void;
  onNodeMoveEnd: (id: string, x: number, y: number) => void;
};

type DragState = { nodeId: string; pointerId: number };

function TopologyCanvas(props: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const movedRef = useRef(false);
  const pendingPositionRef = useRef<{ nodeId: string; x: number; y: number } | null>(null);
  const animationFrameRef = useRef<number | null>(null);
  const nodeMap = useMemo(() => new Map(props.nodes.map((node) => [node.node_id, node])), [props.nodes]);
  const renderedDrawings = useMemo(() => props.drawings.flatMap((drawing) => {
    const markup = typeof drawing.svg === "string" ? sanitizeDrawingSvg(drawing.svg) : "";
    return markup ? [{ drawing, markup }] : [];
  }), [props.drawings]);
  const view = {
    width: scene.width / props.zoom,
    height: scene.height / props.zoom,
    x: -scene.width / props.zoom / 2,
    y: -scene.height / props.zoom / 2,
  };

  useEffect(() => () => {
    if (animationFrameRef.current !== null) window.cancelAnimationFrame(animationFrameRef.current);
  }, []);

  const pointerScenePoint = (event: PointerEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const scale = Math.min(rect.width / view.width, rect.height / view.height);
    const offsetX = (rect.width - view.width * scale) / 2;
    const offsetY = (rect.height - view.height * scale) / 2;
    return {
      x: view.x + (event.clientX - rect.left - offsetX) / scale,
      y: view.y + (event.clientY - rect.top - offsetY) / scale,
    };
  };

  function startMove(event: PointerEvent<SVGGElement>, node: Node) {
    if (props.connecting || event.button !== 0) return;
    event.stopPropagation();
    dragRef.current = { nodeId: node.node_id, pointerId: event.pointerId };
    movedRef.current = false;
    svgRef.current?.setPointerCapture(event.pointerId);
  }

  function moveNode(event: PointerEvent<SVGSVGElement>) {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const point = pointerScenePoint(event);
    const x = Math.round(point.x / 10) * 10;
    const y = Math.round(point.y / 10) * 10;
    movedRef.current = true;
    pendingPositionRef.current = { nodeId: drag.nodeId, x, y };
    if (animationFrameRef.current === null) {
      animationFrameRef.current = window.requestAnimationFrame(() => {
        animationFrameRef.current = null;
        const pending = pendingPositionRef.current;
        if (pending) props.onNodeMove(pending.nodeId, pending.x, pending.y);
      });
    }
  }

  function endMove(event: PointerEvent<SVGSVGElement>) {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    dragRef.current = null;
    if (movedRef.current) {
      const point = pointerScenePoint(event);
      const x = Math.round(point.x / 10) * 10;
      const y = Math.round(point.y / 10) * 10;
      if (animationFrameRef.current !== null) {
        window.cancelAnimationFrame(animationFrameRef.current);
        animationFrameRef.current = null;
      }
      pendingPositionRef.current = null;
      props.onNodeMove(drag.nodeId, x, y);
      props.onNodeMoveEnd(drag.nodeId, x, y);
    }
  }

  return (
    <div className={`topology-canvas ${props.connecting ? "connect-mode" : ""}`}>
      <svg ref={svgRef} viewBox={`${view.x} ${view.y} ${view.width} ${view.height}`} role="group" aria-label="Network topology canvas" onPointerMove={moveNode} onPointerUp={endMove} onPointerCancel={endMove}>
        <defs><pattern id="grid-dots" width="24" height="24" patternUnits="userSpaceOnUse"><circle cx="1" cy="1" r="1" fill="#dce2ed" /></pattern></defs>
        <rect x={view.x} y={view.y} width={view.width} height={view.height} fill="url(#grid-dots)" />
        {renderedDrawings.map(({ drawing, markup }) => <g key={drawing.drawing_id} transform={`translate(${Number(drawing.x ?? 0)},${Number(drawing.y ?? 0)}) rotate(${Number(drawing.rotation ?? 0)})`} style={{ pointerEvents: "none" }} dangerouslySetInnerHTML={{ __html: markup }} />)}
        {props.links.map((link) => {
          const [source, target] = link.nodes ?? [];
          const sourceNode = source && nodeMap.get(source.node_id);
          const targetNode = target && nodeMap.get(target.node_id);
          if (!sourceNode || !targetNode) return null;
          const selected = props.selection?.kind === "link" && props.selection.id === link.link_id;
          return <g key={link.link_id} className={`link-group ${selected ? "selected" : ""}`} onClick={() => props.onLinkClick(link.link_id)} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); props.onLinkClick(link.link_id); } }} role="button" aria-label={`Link ${sourceNode.name} to ${targetNode.name}`} tabIndex={0}>
            <line x1={Number(sourceNode.x ?? 0)} y1={Number(sourceNode.y ?? 0)} x2={Number(targetNode.x ?? 0)} y2={Number(targetNode.y ?? 0)} />
            <line className="link-hit-area" x1={Number(sourceNode.x ?? 0)} y1={Number(sourceNode.y ?? 0)} x2={Number(targetNode.x ?? 0)} y2={Number(targetNode.y ?? 0)} />
          </g>;
        })}
        {props.nodes.map((node) => {
          const x = Number(node.x ?? 0);
          const y = Number(node.y ?? 0);
          const selected = props.selection?.kind === "node" && props.selection.id === node.node_id;
          const connecting = props.firstEndpoint === node.node_id;
          return <g key={node.node_id} transform={`translate(${x},${y})`} className={`topology-node ${selected ? "selected" : ""} ${connecting ? "connect-origin" : ""}`} onPointerDown={(event) => startMove(event, node)} onClick={(event) => { event.stopPropagation(); if (!movedRef.current) props.onNodeClick(node.node_id); movedRef.current = false; }} onKeyDown={(event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); props.onNodeClick(node.node_id); } }} role="button" aria-label={`Node ${node.name}`} tabIndex={0}>
            <circle className="node-shadow" r="27" />
            <circle className="node-shape" r="25" />
            <text className="node-icon" x="0" y="6">▣</text>
            <text className="node-name" x="0" y="48">{node.name}</text>
            <circle className={`node-status ${node.status === "started" ? "running" : "stopped"}`} cx="18" cy="-18" r="4" />
          </g>;
        })}
        {props.nodes.length === 0 && <g className="canvas-empty" transform="translate(0,0)"><circle r="38" /><text className="canvas-empty-icon" y="9">＋</text><text className="canvas-empty-title" y="74">Your canvas is ready</text><text className="canvas-empty-copy" y="98">Add a node to start mapping your network.</text></g>}
      </svg>
      <div className="canvas-coordinate">x {scene.x} — {scene.x + scene.width} · y {scene.y} — {scene.y + scene.height}</div>
    </div>
  );
}

function errorMessage(reason: unknown): string {
  return reason instanceof Error ? reason.message : "Something went wrong. Please try again.";
}

export default App;
