import { describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import "./App.test-utils";
import { api } from "./api";
import { project } from "./App.test-utils";
import { emptyTopology } from "./App.test-utils";
import { FakeWebSocket } from "./App.test-utils";
import { renderRoutedApp } from "./App.test-utils";
import { renderWorkspace } from "./App.test-utils";
import { ProjectNavigationHost } from "./App.test-utils";

describe("projects", () => {
    it("creates a project, opens it, and loads a snapshot-backed canvas", async () => {
      render(<ProjectNavigationHost />);
      await screen.findByRole("textbox", { name: "New project name" }, { timeout: 10_000 });
      fireEvent.change(screen.getByRole("textbox", { name: "New project name" }), {
        target: { value: "Core lab" },
      });
      fireEvent.click(screen.getByRole("button", { name: "Create project" }));
  
      expect(await screen.findByText("Your canvas is ready", {}, { timeout: 10_000 })).toBeInTheDocument();
      expect(api.createProject).toHaveBeenCalledWith("Core lab");
      await waitFor(() => expect(api.openProject).toHaveBeenCalledWith("p1"), { timeout: 10_000 });
      expect(api.topology).toHaveBeenCalledWith("p1");
    }, 15_000);

    it("opens an existing project and shows its nodes from the topology snapshot", async () => {
      vi.mocked(api.listProjects).mockResolvedValue([project]);
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "n1", name: "Edge router", node_type: "router", x: 0, y: 0 }],
      });
      renderRoutedApp();
  
      fireEvent.click(await screen.findByRole("button", { name: /Open project/ }));
  
      expect(await screen.findByRole("button", { name: "Node Edge router" })).toBeInTheDocument();
  
      act(() => {
        FakeWebSocket.last?.onmessage?.({
          data: JSON.stringify({
            project_id: "p1",
            sequence: 8,
            action: "node.created",
            event: { node_id: "n2", name: "Distribution switch", node_type: "ethernet_switch", x: 220, y: 100 },
          }),
        } as MessageEvent);
      });
      expect(await screen.findByRole("button", { name: "Node Distribution switch" }, { timeout: 10_000 })).toBeInTheDocument();
    });

    it("opens a console for a running Dynamips IOS router", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "ios-1", name: "R1", node_type: "dynamips", status: "started", x: 0, y: 0 }],
      });
      renderWorkspace();
  
      fireEvent.click(await screen.findByRole("button", { name: "Node R1" }));
      fireEvent.click(await screen.findByRole("button", { name: "Open console" }));
  
      expect(await screen.findByRole("region", { name: "Console for R1" }, { timeout: 10_000 })).toBeInTheDocument();
      expect(screen.getByText("Dynamips IOS console")).toBeInTheDocument();
    });

    it("opens the configured Docker auxiliary console through its WebSocket route", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "docker-aux", name: "Aux host", node_type: "docker", status: "started", aux_type: "telnet", aux_console_port: 5001, x: 0, y: 0 }],
      });
      renderWorkspace();
  
      fireEvent.contextMenu(await screen.findByRole("button", { name: "Node Aux host" }));
      fireEvent.click(await screen.findByRole("menuitem", { name: "Open auxiliary console" }));
  
      expect(await screen.findByRole("region", { name: "Auxiliary console for Aux host" }, { timeout: 10_000 })).toBeInTheDocument();
      expect(screen.getByText("Docker auxiliary console")).toBeInTheDocument();
    });

    it("redirects project links saved with the previous hash URL format", async () => {
      window.location.hash = "#/projects/p1";
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "n1", name: "Edge router", node_type: "router", x: 0, y: 0 }],
      });
      const router = renderRoutedApp();
  
      expect(await screen.findByRole("button", { name: "Node Edge router" })).toBeInTheDocument();
      expect(router.state.location.pathname).toBe("/projects/p1");
    });

    it("restores a project from a direct typed-route URL", async () => {
      vi.mocked(api.topology).mockResolvedValue({
        ...emptyTopology,
        nodes: [{ node_id: "n1", name: "Edge router", node_type: "router", x: 0, y: 0 }],
      });
      const router = renderRoutedApp("/projects/p1");
  
      expect(await screen.findByRole("button", { name: "Node Edge router" })).toBeInTheDocument();
      expect(router.state.location.pathname).toBe("/projects/p1");
      expect(api.openProject).toHaveBeenCalledWith("p1");
    });

});
