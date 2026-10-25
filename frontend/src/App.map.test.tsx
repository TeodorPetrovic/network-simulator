import { describe, expect, it, vi } from "vitest";
import { fireEvent, screen, waitFor } from "@testing-library/react";
import "./App.test-utils";
import { api } from "./api";
import { emptyTopology } from "./App.test-utils";
import { renderWorkspace } from "./App.test-utils";

describe("map", () => {
    it("offers packet capture directly from the selected link inspector", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [
          { node_id: "n1", name: "Router A", node_type: "qemu", x: 0, y: 0 },
          { node_id: "n2", name: "Router B", node_type: "qemu", x: 200, y: 0 },
        ],
        links: [{ link_id: "l1", nodes: [
          { node_id: "n1", adapter_number: 0, port_number: 0 },
          { node_id: "n2", adapter_number: 0, port_number: 0 },
        ] }],
      });
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: "Ethernet segment connecting Router A, Router B" }));

      expect(await screen.findByRole("button", { name: "Packet capture" })).toBeInTheDocument();
    });

    it("keeps zoom controls out of the canvas scene", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "n1", name: "Edge router", node_type: "qemu", x: 180, y: 90 }],
      });
      renderWorkspace();
      await screen.findByRole("button", { name: "Node Edge router" });
      expect(screen.getByRole("button", { name: "Project map menu" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Zoom in" })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Zoom to selection" })).not.toBeInTheDocument();
    });

    it("shows live host metrics from the topology map resource control", async () => {
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: /Resources/ }));
  
      expect(await screen.findByText("Resource monitor")).toBeInTheDocument();
      expect(screen.getByText("12.5%", { exact: true })).toBeInTheDocument();
      expect(screen.getByText("Controller RSS")).toBeInTheDocument();
    });

    it("starts every stopped device from the workspace toolbar", async () => {
      const nodes = [
        { node_id: "n1", name: "Edge A", node_type: "docker", status: "stopped", x: 0, y: 0 },
        { node_id: "n2", name: "Edge B", node_type: "docker", status: "started", x: 120, y: 0 },
      ];
      vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
      vi.mocked(api.startNode).mockResolvedValue({ ...nodes[0], status: "started" });
      renderWorkspace();
  
      fireEvent.click(await screen.findByRole("button", { name: "Start all" }));
  
      await waitFor(() => expect(api.startNode).toHaveBeenCalledTimes(1));
      expect(api.startNode).toHaveBeenCalledWith("p1", "n1");
    });

    it("auto-names a device added from the right-hand device library", async () => {
      const template = {
        template_id: "qemu-template-1",
        name: "router.qcow2",
        template_type: "qemu" as const,
        hda_disk_image: "/images/qemu/router.qcow2",
        adapters: 4,
        memory: 512,
        cpus: 1,
      };
      vi.mocked(api.templates).mockResolvedValue([template]);
      vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
        node_id: "vm-1",
        name: "router 1",
        node_type: "qemu",
        status: "stopped",
      });
      renderWorkspace();
  
      fireEvent.click(await screen.findByRole("button", { name: "Add router.qcow2" }));
  
      await waitFor(() => expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "qemu-template-1", {
        name: "router 1",
        x: -420,
        y: -220,
      }));
    });

    it("opens Docker and QEMU device groups when runtime templates finish loading", async () => {
      vi.mocked(api.templates).mockResolvedValue([
        { template_id: "docker-template", name: "alpine:latest", template_type: "docker", adapters: 2, image: "alpine:latest", image_digest: "sha256:test", memory: 512, cpus: 1 },
        { template_id: "qemu-template", name: "router.qcow2", template_type: "qemu", adapters: 4, hda_disk_image: "/images/router.qcow2", memory: 512, cpus: 1 },
      ]);
      renderWorkspace();
  
      expect(await screen.findByRole("button", { name: "Add alpine:latest" })).toBeInTheDocument();
      expect(await screen.findByRole("button", { name: "Add router.qcow2" })).toBeInTheDocument();
      expect(screen.getByText("Docker")).toBeInTheDocument();
      expect(screen.getByText("QEMU / KVM")).toBeInTheDocument();
    });

    it("opens an interactive serial console for a running QEMU node", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "vm-1", name: "Router VM", node_type: "qemu", status: "started", x: 0, y: 0 }],
      });
      renderWorkspace();
  
      fireEvent.click(await screen.findByRole("button", { name: "Node Router VM" }));
      fireEvent.click(await screen.findByRole("button", { name: "Open console" }));
  
      expect(await screen.findByRole("region", { name: "Console for Router VM" }, { timeout: 10_000 })).toBeInTheDocument();
      expect(screen.getByText("QEMU serial console")).toBeInTheDocument();
    });

    it("creates a Dynamips c7200 node with its IOS startup settings", async () => {
      const iosTemplate = {
        template_id: "ios-template",
        name: "c7200-adventerprisek9.bin · c7200",
        template_type: "dynamips" as const,
        platform: "c7200" as const,
        image: "/images/dynamips/c7200-adventerprisek9.bin",
        adapters: 1 as const,
        memory: 256,
        nvram: 128,
        idlepc: "",
      };
      vi.mocked(api.templates).mockResolvedValue([iosTemplate]);
      vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
        node_id: "ios-1",
        name: "R1",
        node_type: "dynamips",
        status: "stopped",
      });
      vi.mocked(api.topology).mockResolvedValue(emptyTopology);
      renderWorkspace();
  
      fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
      fireEvent.change(await screen.findByRole("textbox", { name: "Node name" }), { target: { value: "R1" } });
      const idlePcInput = screen.getByRole("textbox", { name: "Idle-PC address (optional)" });
      expect(idlePcInput).toHaveAttribute("placeholder", "Use the appliance default");
      fireEvent.change(idlePcInput, { target: { value: "0x606e0538" } });
      fireEvent.change(screen.getByRole("textbox", { name: "IOS startup configuration (optional)" }), { target: { value: "hostname R1\nno ip domain-lookup" } });
      fireEvent.click(screen.getByRole("button", { name: "Create node" }));
  
      await waitFor(() => expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "ios-template", expect.objectContaining({
        name: "R1",
        idlepc: "0x606e0538",
        startup_config: "hostname R1\nno ip domain-lookup",
      })));
    });

    it("opens an attached terminal for a running Docker node", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "docker-1", name: "Alpine host", node_type: "docker", status: "started", x: 0, y: 0 }],
      });
      renderWorkspace();
  
      fireEvent.click(await screen.findByRole("button", { name: "Node Alpine host" }));
      fireEvent.click(await screen.findByRole("button", { name: "Open console" }));
  
      expect(await screen.findByRole("region", { name: "Console for Alpine host" }, { timeout: 10_000 })).toBeInTheDocument();
      expect(screen.getByText("Docker attached console")).toBeInTheDocument();
    });

    it("creates a VPCS node with its real startup script template field", async () => {
      const template = {
        template_id: "vpcs-template-1",
        name: "VPCS",
        template_type: "vpcs" as const,
        adapters: 1 as const,
        startup_script: "",
      };
      vi.mocked(api.templates).mockResolvedValue([template]);
      vi.mocked(api.createNodeFromTemplate).mockResolvedValue({
        node_id: "pc-1",
        name: "PC A",
        node_type: "vpcs",
        status: "stopped",
        x: -420,
        y: -210,
      });
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: /Add node/ }));
      expect(await screen.findByRole("textbox", { name: "Node name" })).toBeInTheDocument();
      fireEvent.change(screen.getByRole("textbox", { name: "Node name" }), {
        target: { value: "PC A" },
      });
      fireEvent.change(screen.getByRole("textbox", { name: "Startup script (optional)" }), {
        target: { value: "ip 10.0.0.1/24" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Create node" }));
      expect(await screen.findByRole("button", { name: "Node PC A" })).toBeInTheDocument();
      expect(api.createNodeFromTemplate).toHaveBeenCalledWith("p1", "vpcs-template-1", {
        name: "PC A",
        x: -420,
        y: -210,
        startup_script: "ip 10.0.0.1/24",
      });
    });

    it("creates a point-to-point Ethernet link between selected ports", async () => {
      const nodes = ["Edge A", "Edge B"].map((name, index) => ({
        node_id: `node-${index + 1}`,
        name,
        node_type: "docker",
        x: index * 120,
        y: 0,
      }));
      vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
      vi.mocked(api.createLink).mockResolvedValue({
        link_id: "link-1",
        nodes: nodes.map((node) => ({ node_id: node.node_id, adapter_number: 0, port_number: 0 })),
      });
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: "Connect ports" }));
      fireEvent.contextMenu(screen.getByRole("button", { name: "Node Edge A" }));
      fireEvent.click(await screen.findByRole("menuitem", { name: /eth0/ }));
      fireEvent.contextMenu(screen.getByRole("button", { name: "Node Edge B" }));
      fireEvent.click(await screen.findByRole("menuitem", { name: /eth0/ }));
  
      await waitFor(() => expect(api.createLink).toHaveBeenCalledWith("p1", [
        { node_id: "node-1", adapter_number: 0, port_number: 0 },
        { node_id: "node-2", adapter_number: 0, port_number: 0 },
      ]));
      expect(await screen.findByText("Ethernet segment created with 2 endpoints")).toBeInTheDocument();
    });

    it("exposes the complete node context-menu actions for a running device", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "n1", name: "Edge router", node_type: "qemu", status: "started", x: 0, y: 0 }],
      });
      renderWorkspace();
      fireEvent.contextMenu(await screen.findByRole("button", { name: "Node Edge router" }));
  
      for (const label of ["Suspend device", "Stop device", "Reload device", "Open console", "Edit device name", "Configure device", "Change symbol", "Isolate device", "Lock device", "Duplicate device", "Delete device"]) {
        expect(await screen.findByRole("menuitem", { name: label })).toBeInTheDocument();
      }
    });

    it("multi-selects devices and exposes bulk lifecycle actions", async () => {
      const nodes = ["Edge A", "Edge B"].map((name, index) => ({
        node_id: `node-${index + 1}`,
        name,
        node_type: "docker",
        status: "stopped",
        x: index * 120,
        y: 0,
      }));
      vi.mocked(api.topology).mockResolvedValue({ ...emptyTopology, nodes });
      vi.mocked(api.startNode).mockImplementation(async (_projectId, nodeId) => ({ node_id: nodeId, name: nodeId === "node-1" ? "Edge A" : "Edge B", node_type: "docker", status: "started" }));
      renderWorkspace();
      fireEvent.click(await screen.findByRole("button", { name: "Node Edge A" }), { ctrlKey: true });
      fireEvent.click(screen.getByRole("button", { name: "Node Edge B" }), { ctrlKey: true });
      expect(screen.getByText("2 selected")).toBeInTheDocument();
      fireEvent.click(screen.getByRole("button", { name: "Start" }));
  
      await waitFor(() => {
        expect(api.startNode).toHaveBeenCalledWith("p1", "node-1");
        expect(api.startNode).toHaveBeenCalledWith("p1", "node-2");
      });
    });

});
