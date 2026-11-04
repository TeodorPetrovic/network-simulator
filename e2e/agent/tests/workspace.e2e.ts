import { test } from '@e2e-dev/web';
import { expect } from 'e2e';

const base = process.env.E2E_BASE_URL ?? 'http://127.0.0.1:3180';
const api = async (method: string, path: string, body?: unknown) => {
  const r = await fetch(`${base}/v3${path}`, { method, headers: { 'content-type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
  return r.json();
};

test('device menu offers Edit config and opens the editor', async ({ app, screen }) => {
  const { project_id } = await api('POST', '/projects', { name: 'agent-e2e' }) as { project_id: string };
  const templates = await api('GET', '/templates') as { template_id: string; template_type: string }[];
  const vpcs = templates.find((t) => t.template_type === 'vpcs')!;
  await api('POST', `/projects/${project_id}/templates/${vpcs.template_id}`, { x: 0, y: 0, compute_id: 'local', name: 'PC1' });
  await app.open(`/projects/${project_id}`);
  await screen.getByLabel('Node PC1').secondaryTap();
  await expect(screen.getByRole('menuitem', 'Edit config')).toBeVisible();
  await screen.getByRole('menuitem', 'Edit config').tap();
  await expect(screen.getByRole('textbox', 'Configuration')).toBeVisible();
});
