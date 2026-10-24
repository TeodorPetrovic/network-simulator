import { useEffect, useRef, useState } from "react";
import { api, projectNotificationsUrl } from "./api";
import type { Project, ProjectEvent, RuntimeTemplate, TopologySnapshot } from "./api";
import { applyProjectEvent } from "./topology";
import { errorMessage } from "./appUtils";

export function useProjectSession(projectId: string | null, setNotice: (message: string) => void) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [project, setProject] = useState<Project | null>(null);
  const [topology, setTopology] = useState<TopologySnapshot | null>(null);
  const [busy, setBusy] = useState(true);
  const [connection, setConnection] = useState<"checking" | "connected" | "reconnecting" | "disconnected">("checking");
  const [error, setError] = useState("");
  const [templates, setTemplates] = useState<RuntimeTemplate[]>([]);
  const socketRef = useRef<WebSocket | null>(null);

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
    const connect = async () => {
      if (!active) return;
      let url: string;
      try {
        url = await projectNotificationsUrl(projectId, cursor);
      } catch (reason) {
        if (active) {
          setConnection("reconnecting");
          setError(errorMessage(reason));
          reconnectTimer = window.setTimeout(() => void connect(), 1200);
        }
        return;
      }
      if (!active) return;
      const socket = new WebSocket(url);
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
          reconnectTimer = window.setTimeout(() => void connect(), 1200);
        }
      };
      socket.onerror = () => socket.close();
    };

    const loadProject = async () => {
      try {
        await api.openProject(projectId);
        const [snapshot, availableTemplates] = await Promise.all([api.topology(projectId), api.templates()]);
        if (!active) return;
        cursor = snapshot.event_cursor;
        setProject(snapshot.project);
        setTopology(snapshot);
        setTemplates(availableTemplates);
        setConnection("connected");
        void connect();
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

  return { projects, setProjects, project, setProject, topology, setTopology, busy, connection, error, setError, templates, setTemplates };
}
