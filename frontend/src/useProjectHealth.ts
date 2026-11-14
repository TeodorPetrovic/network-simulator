import { useEffect, useState } from "react";
import { api } from "./api";
import type { Project } from "./api";
import { projectHealth, type ProjectHealth } from "./projectHealth";

type HealthSnapshot = { health?: ProjectHealth; checkedAt?: number; error?: string };

/** Fetch at most four topologies at once; pause periodic checks in hidden tabs. */
export function useProjectHealth(projects: Project[]) {
  const [snapshots, setSnapshots] = useState<Record<string, HealthSnapshot>>({});
  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout>;
    let inFlight = false;
    setSnapshots({});
    const check = async () => {
      if (inFlight || !active) return;
      clearTimeout(timer);
      inFlight = true;
      if (!document.hidden) {
        let index = 0;
        await Promise.all(Array.from({ length: Math.min(4, projects.length) }, async () => {
          while (active && !document.hidden && index < projects.length) {
            const project = projects[index++];
            let snapshot: HealthSnapshot;
            try { snapshot = { health: projectHealth(await api.topology(project.project_id)), checkedAt: Date.now() }; }
            catch { snapshot = { error: "Health could not be checked. Open the project or refresh to try again." }; }
            if (active) setSnapshots((current) => ({ ...current, [project.project_id]: snapshot }));
          }
        }));
      }
      inFlight = false;
      if (active) timer = setTimeout(() => void check(), 30_000);
    };
    const onVisibility = () => { if (!document.hidden && projects.length) void check(); };
    document.addEventListener("visibilitychange", onVisibility);
    if (projects.length) void check();
    return () => { active = false; clearTimeout(timer); document.removeEventListener("visibilitychange", onVisibility); };
  }, [projects]);
  return snapshots;
}
