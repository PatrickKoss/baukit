import { act, fireEvent, render, screen } from '@testing-library/react-native';
import type { ReactElement } from 'react';
import { AccessibilityInfo, Text } from 'react-native';

import { createApiAvailabilityStore } from './api-availability';
import { formatRetryTime } from './api-status';
import { ApiUnavailableBanner, AppErrorBoundary } from './app-status';
import { initializeI18n } from './localization/i18n';

jest.mock('./theme', () => {
  const theme = jest.requireActual<typeof import('./theme')>('./theme');
  return { ...theme, useTheme: () => ({ theme: theme.lightTheme }) };
});

const RETRY_AT = Date.UTC(2030, 0, 1, 14, 5);

beforeEach(async () => {
  await initializeI18n('en');
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe('ApiUnavailableBanner', () => {
  it('shows and announces the retry time, then disappears', async () => {
    const spoken = jest.spyOn(AccessibilityInfo, 'announceForAccessibility').mockImplementation();
    const store = createApiAvailabilityStore();
    await render(<ApiUnavailableBanner store={store} />);
    expect(screen.queryByText(/unavailable/u)).toBeNull();

    await act(() => {
      store.markUnavailable(RETRY_AT);
    });
    const message = `The service is unavailable. Retrying at ${formatRetryTime(RETRY_AT, 'en')}.`;
    expect(screen.getByText(message)).toBeOnTheScreen();
    expect(spoken).toHaveBeenCalledWith(message);

    await act(() => {
      store.markAvailable();
    });
    expect(screen.queryByText(message)).toBeNull();
  });

  it('uses the German catalog', async () => {
    await initializeI18n('de');
    const store = createApiAvailabilityStore();
    store.markUnavailable(RETRY_AT);
    await render(<ApiUnavailableBanner store={store} />);

    expect(
      screen.getByText(`Der Dienst ist nicht erreichbar. Neuer Versuch um ${formatRetryTime(RETRY_AT, 'de')}.`),
    ).toBeOnTheScreen();
    store.markAvailable();
  });
});

function Flaky({ crash }: { readonly crash: () => boolean }): ReactElement {
  if (crash()) throw new Error('render failed');
  return <Text>Recovered</Text>;
}

describe('AppErrorBoundary', () => {
  it('announces the crash and recovers on retry', async () => {
    jest.spyOn(console, 'error').mockImplementation(() => undefined);
    const spoken = jest.spyOn(AccessibilityInfo, 'announceForAccessibility').mockImplementation();
    let broken = true;
    await render(
      <AppErrorBoundary>
        <Flaky crash={() => broken} />
      </AppErrorBoundary>,
    );

    expect(spoken).toHaveBeenCalledWith('Something went wrong');
    expect(screen.getByRole('header', { name: 'Something went wrong' })).toBeOnTheScreen();
    broken = false;
    await fireEvent.press(screen.getByRole('button', { name: 'Try again' }));
    expect(screen.getByText('Recovered')).toBeOnTheScreen();
  });
});
