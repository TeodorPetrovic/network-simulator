import type { E2EConfig } from 'e2e';
import { web } from '@e2e-dev/web';
import { chatgpt } from 'e2e/oauth/chatgpt';

export default {
  targets: [{
    engine: web(),
    app: { url: process.env.E2E_BASE_URL ?? 'http://127.0.0.1:3180' },
  }],
  agents: {
    default: {
      model: chatgpt('gpt-6-luna'),
      // high reasoning; no serviceTier set => default (not priority/"speed")
      providerOptions: { openai: { reasoningEffort: 'high' } },
    },
  },
} satisfies E2EConfig;
