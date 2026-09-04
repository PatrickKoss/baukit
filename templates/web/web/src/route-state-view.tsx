import type { ReactNode } from 'react';

import type { DetailRouteState } from './route-state';

interface DetailRouteStateViewProps<T> {
  readonly state: DetailRouteState<T>;
  readonly onExit: () => void;
  readonly renderReady: (value: T) => ReactNode;
}

export function DetailRouteStateView<T>({
  state,
  onExit,
  renderReady,
}: DetailRouteStateViewProps<T>) {
  if (state.status === 'ready') {
    return renderReady(state.value);
  }

  const content = {
    loading: ['Loading detail', 'Please wait while the detail loads.'],
    invalid: ['Invalid link', 'This link does not contain a valid detail identifier.'],
    'not-found': ['Detail not found', 'The requested detail is no longer available.'],
    error: ['Could not load detail', state.status === 'error' ? state.message : ''],
  }[state.status];

  return (
    <section
      className="panel route-state"
      aria-busy={state.status === 'loading'}
      aria-live={state.status === 'error' ? 'assertive' : 'polite'}
      role={state.status === 'error' ? 'alert' : 'status'}
    >
      <h2>{content[0]}</h2>
      <p className={state.status === 'error' ? 'error' : 'muted'}>{content[1]}</p>
      {state.status === 'loading' ? null : state.status === 'not-found' ? (
        <a
          className="action secondary"
          href="/"
          onClick={(event) => {
            event.preventDefault();
            onExit();
          }}
        >
          Back to items
        </a>
      ) : (
        <button className="action secondary" type="button" onClick={onExit}>
          Back to items
        </button>
      )}
    </section>
  );
}
