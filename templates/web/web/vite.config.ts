import react from '@vitejs/plugin-react';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [react()],
  server: {
    port: {{ context.web_port }},
  },
{% if context.web_preview_declared %}  preview: {
    port: {{ context.web_preview_port }},
  },
{% endif %}});
