import { Component, type ReactElement, type ReactNode } from 'react';

import { apiAvailability, type ApiAvailabilityStore } from './api-availability';
import { statusCopy, unavailableMessage, useApiAvailability } from './api-status';

/** Announces the API cooldown politely; renders nothing while the API is reachable. */
export function ApiUnavailableBanner({
  store = apiAvailability,
}: {
  readonly store?: ApiAvailabilityStore;
}): ReactElement {
  const retryAt = useApiAvailability(store);
  return (
    <div role="status">
      {retryAt === undefined ? null : <p className="status-banner">{unavailableMessage(retryAt)}</p>}
    </div>
  );
}

interface ErrorBoundaryState {
  readonly failed: boolean;
}

export class AppErrorBoundary extends Component<{ readonly children: ReactNode }, ErrorBoundaryState> {
  public override state: ErrorBoundaryState = { failed: false };

  public static getDerivedStateFromError(): ErrorBoundaryState {
    return { failed: true };
  }

  public override render(): ReactNode {
    if (!this.state.failed) return this.props.children;
    return <AppCrashed />;
  }
}

function AppCrashed(): ReactElement {
  const copy = statusCopy();
  return (
    <main className="shell">
      <section className="panel" role="alert" aria-labelledby="app-crashed-title">
        <h1 id="app-crashed-title">{copy.crashed}</h1>
        <p>{copy.crashedDetail}</p>
        <button
          type="button"
          onClick={() => {
            window.location.reload();
          }}
        >
          {copy.reload}
        </button>
      </section>
    </main>
  );
}
