import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentType } from "react";
import { afterEach, beforeEach, describe, expect, it as registerTest, vi } from "vitest";
import { api } from "./api";
import type { Project, RuntimeInfo, RuntimeTemplate, TopologySnapshot } from "./api";
import { ProjectsPage } from "./ProjectsPage";
import { PreferencesPage } from "./PreferencesPage";
import { SettingsPage } from "./SettingsPage";
import { SystemStatusPage } from "./SystemStatusPage";
import { InstalledSoftwarePage } from "./InstalledSoftwarePage";
import { ComputesPage } from "./ComputesPage";

const it = (name: string, fn: any, timeout = 15_000) => registerTest(name, fn, timeout);

vi.mock("./PagesShell", async () => {
  const React = await import("react");
  return {
    PagesShell: ({ children }: { children: import("react").ReactNode }) => React.createElement("main", null, children),
    PageTitle: ({ title, description, actions }: { title: string; description?: string; actions?: import("react").ReactNode }) => React.createElement("header", null, React.createElement("h1", null, title), description && React.createElement("p", null, description), actions),
  };
});

vi.mock("@tanstack/react-router", () => ({ useNavigate: () => vi.fn() }));

vi.mock("./api", () => ({ api: {
  listProjects: vi.fn(), createProject: vi.fn(), deleteProject: vi.fn(), updateProject: vi.fn(), duplicateProject: vi.fn(), projectStatistics: vi.fn(), projectReadme: vi.fn(), updateProjectReadme: vi.fn(), exportProject: vi.fn(), importProject: vi.fn(), openProject: vi.fn(), closeProject: vi.fn(),
  settings: vi.fn(), updateSettings: vi.fn(), computes: vi.fn(),
  templates: vi.fn(), updateTemplate: vi.fn(), createTemplate: vi.fn(), duplicateTemplate: vi.fn(), deleteTemplate: vi.fn(),
  runtimes: vi.fn(), resourceMonitor: vi.fn(), topology: vi.fn(), configureNode: vi.fn(), snapshots: vi.fn(), createSnapshot: vi.fn(), restoreSnapshot: vi.fn(), deleteSnapshot: vi.fn(),
  searchGns3Appliances: vi.fn(), gns3Appliance: vi.fn(), importGns3Appliance: vi.fn(), importGns3ApplianceFile: vi.fn(),
} }));

const project: Project = { project_id: "project-1", name: "Core lab", status: "closed", created_at: "2026-10-01T00:00:00Z" };
const qemuTemplate: RuntimeTemplate = { template_id: "qemu-1", name: "Router VM", template_type: "qemu", hda_disk_image: "router.qcow2", adapters: 4, memory: 1024, cpus: 1, hda_disk_interface: "virtio", adapter_type: "e1000", boot_priority: "c" };
const runtimes: RuntimeInfo = {
  docker: { available: true, version: "29.0", error: null },
  qemu: { available: true, version: "9.0", qemu_img_available: true, image_directory: "/images/qemu", image_count: 1, acceleration: "KVM", kvm_accessible: true, acceleration_error: null, error: null },
  vpcs: { available: false, version: null, error: "VPCS missing" },
  dynamips: { available: false, version: null, binary: "dynamips", image_directory: "/images/dynamips", image_count: 0, error: "Dynamips missing" },
  ubridge: { available: true, binary: "ubridge", version: "0.9", error: null },
};
const resources = { sampled_at_unix_ms: 1_797_000_000_000, host: { cpu_usage_percent: 22, cpu_count: 8, memory_used_bytes: 4_000_000_000, memory_total_bytes: 16_000_000_000, load_average: [0.1, 0.2, 0.3] }, controller: { memory_rss_bytes: 100_000_000, thread_count: 8 } };
const topology: TopologySnapshot = { project, nodes: [{ node_id: "hub-1", node_type: "ethernet_hub", name: "Hub 1", status: "stopped" }], links: [], drawings: [], event_cursor: 0 };

function renderPage(Page: ComponentType) {
  render(<Page />);
}

afterEach(() => { cleanup(); });
beforeEach(() => {
  vi.mocked(api.listProjects).mockResolvedValue([project]);
  vi.mocked(api.createProject).mockResolvedValue(project);
  vi.mocked(api.importProject).mockResolvedValue({ ...project, project_id: "imported-1", name: "Imported lab" });
  vi.mocked(api.duplicateProject).mockResolvedValue({ ...project, project_id: "copy-1", name: "Core lab copy" });
  vi.mocked(api.deleteProject).mockResolvedValue(undefined);
  vi.mocked(api.templates).mockResolvedValue([]);
  vi.mocked(api.runtimes).mockResolvedValue(runtimes);
  vi.mocked(api.resourceMonitor).mockResolvedValue(resources);
  vi.mocked(api.computes).mockResolvedValue([]);
  vi.mocked(api.settings).mockResolvedValue({ Server: { host: "127.0.0.1", port: 3080, crash_reports: false } });
  vi.mocked(api.updateSettings).mockImplementation(async (settings) => settings);
  vi.mocked(api.topology).mockResolvedValue(topology);
  vi.mocked(api.configureNode).mockResolvedValue(topology.nodes[0]);
});

describe("PAGES parity routes", () => {
  it("filters the project table by name", async () => {
    vi.mocked(api.listProjects).mockResolvedValue([project, { ...project, project_id: "project-2", name: "Branch office" }]);
    renderPage(ProjectsPage);
    expect(await screen.findByText("Core lab")).toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Search projects by name" }), { target: { value: "branch" } });
    expect(screen.getByText("Branch office")).toBeInTheDocument();
    expect(screen.queryByText("Core lab")).not.toBeInTheDocument();
  });

  it("imports a selected .gns3project archive and uses its destination name", async () => {
    renderPage(ProjectsPage);
    fireEvent.click(await screen.findByRole("button", { name: "Import .gns3project" }));
    const file = new File(["archive"], "Imported lab.gns3project", { type: "application/octet-stream" });
    fireEvent.change(screen.getByLabelText("Project archive"), { target: { files: [file] } });
    fireEvent.click(screen.getByRole("button", { name: "Import project" }));
    await waitFor(() => expect(api.importProject).toHaveBeenCalledWith(expect.any(String), "Imported lab", file));
    expect(await screen.findByRole("status")).toHaveTextContent("Imported “Imported lab”.");
  });

  it("lets a user create a README when the project has no README file yet", async () => {
    vi.mocked(api.projectReadme).mockResolvedValue({ content: "" });
    vi.mocked(api.updateProjectReadme).mockResolvedValue(undefined);
    renderPage(ProjectsPage);
    fireEvent.click(await screen.findByRole("button", { name: "Readme for Core lab" }));
    const content = await screen.findByRole("textbox", { name: "Project readme content" });
    fireEvent.change(content, { target: { value: "First project notes" } });
    fireEvent.click(screen.getByRole("button", { name: "Save readme" }));
    await waitFor(() => expect(api.updateProjectReadme).toHaveBeenCalledWith("project-1", "First project notes"));
    expect(await screen.findByRole("status")).toHaveTextContent("Project readme saved.");
  });

  it("shows unsupported runtime states and labels non-consumed template fields", async () => {
    vi.mocked(api.templates).mockResolvedValue([qemuTemplate]);
    renderPage(PreferencesPage);
    fireEvent.click(await screen.findByRole("button", { name: "QEMU" }));
    fireEvent.click(screen.getByRole("button", { name: "Configure" }));
    expect(await screen.findAllByText("Pending backend")).not.toHaveLength(0);
    expect(screen.getAllByText("Pending backend").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(screen.getByRole("button", { name: /IOU/ }));
    expect(screen.getByText("IOU is unsupported")).toBeInTheDocument();
  });

  it("renders only controller-returned settings and saves those fields", async () => {
    renderPage(SettingsPage);
    const host = await screen.findByLabelText("Host");
    expect(host).toHaveValue("127.0.0.1");
    expect(screen.queryByText("Send anonymous crash reports")).not.toBeInTheDocument();
    fireEvent.change(host, { target: { value: "0.0.0.0" } });
    fireEvent.click(screen.getByRole("button", { name: "Save settings" }));
    await waitFor(() => expect(api.updateSettings).toHaveBeenCalledWith({ Server: { host: "0.0.0.0", port: 3080, crash_reports: false } }));
  });

  it("shows live status metrics and installed binary availability", async () => {
    renderPage(SystemStatusPage);
    expect(await screen.findByText("22.0%", { exact: true })).toBeInTheDocument();
    expect(await screen.findByText("Runtime availability")).toBeInTheDocument();
    cleanup();
    renderPage(InstalledSoftwarePage);
    expect(await screen.findByText("Docker Engine")).toBeInTheDocument();
    const installButtons = screen.getAllByRole("button", { name: /Install unavailable/ });
    expect(installButtons.length).toBeGreaterThan(0);
    installButtons.forEach((button) => expect(button).toBeDisabled());
  });

  it("reports compute management as unavailable when the backend route is not ready", async () => {
    vi.mocked(api.computes).mockRejectedValue(new Error("Request failed (404)"));
    renderPage(ComputesPage);
    expect(await screen.findByText(/Compute management unavailable/)).toBeInTheDocument();
    expect(screen.getByText("Local runtime status")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Add remote compute" })).not.toBeInTheDocument();
  });
});
