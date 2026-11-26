import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { QueryClientProvider } from '@tanstack/react-query';

import { App } from './App';
import { ApiUnavailableBanner, AppErrorBoundary } from './app-status';
import { createAppQueryClient } from './query-client';
import './styles.css';

const root = document.querySelector<HTMLDivElement>('#root');
if (root === null) {
  throw new Error('Missing #root element.');
}

const queryClient = createAppQueryClient();

createRoot(root).render(
  <StrictMode>
    <AppErrorBoundary>
      <QueryClientProvider client={queryClient}>
        <ApiUnavailableBanner />
        <App />
      </QueryClientProvider>
    </AppErrorBoundary>
  </StrictMode>,
);
