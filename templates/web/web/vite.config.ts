import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

import { policyOrigins } from './csp/origins.ts';
import { contentSecurityPolicyPlugin } from './csp/vite-plugin.ts';

export default defineConfig({
  plugins: [react(), contentSecurityPolicyPlugin({ styles: 'files', origins: policyOrigins })],
  server: {
    port: {{ context.web_port }},
  },
{% if context.web_preview_declared %}  preview: {
    port: {{ context.web_preview_port }},
  },
{% endif %}});
