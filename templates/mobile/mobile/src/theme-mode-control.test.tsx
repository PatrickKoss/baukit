import { fireEvent, render, screen } from '@testing-library/react-native';

import type { ThemePreference } from './app-preferences';
import { AppThemeProvider } from './theme';
import { ThemeModeControl } from './theme-mode-control';

function renderControl(
  mode: ThemePreference,
  persistMode: (mode: ThemePreference) => Promise<void>,
) {
  return render(
    <AppThemeProvider mode={mode} persistMode={persistMode}>
      <ThemeModeControl />
    </AppThemeProvider>,
  );
}

describe('ThemeModeControl', () => {
  it('marks the current mode as the checked radio', async () => {
    await renderControl('dark', () => Promise.resolve());

    expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked();
    expect(screen.getByRole('radio', { name: 'Light' })).not.toBeChecked();
    expect(screen.getByRole('radio', { name: 'System' })).not.toBeChecked();
  });

  it('persists the pressed mode', async () => {
    const persistMode = jest.fn(() => Promise.resolve());
    await renderControl('system', persistMode);

    await fireEvent.press(screen.getByRole('radio', { name: 'Light' }));

    expect(persistMode).toHaveBeenCalledWith('light');
    expect(screen.queryByText(/Could not/)).toBeNull();
  });

  it('announces the save error and clears it on the next attempt', async () => {
    const persistMode = jest
      .fn<Promise<void>, [ThemePreference]>()
      .mockRejectedValueOnce(new Error('Storage is full.'))
      .mockResolvedValueOnce(undefined);
    await renderControl('system', persistMode);

    await fireEvent.press(screen.getByRole('radio', { name: 'Dark' }));
    expect(await screen.findByText('Storage is full.')).toBeOnTheScreen();

    await fireEvent.press(screen.getByRole('radio', { name: 'Dark' }));
    expect(screen.queryByText('Storage is full.')).toBeNull();
  });

  it('falls back to a generic message for a non-Error rejection', async () => {
    await renderControl(
      'system',
      jest.fn<Promise<void>, [ThemePreference]>().mockRejectedValue('offline'),
    );

    await fireEvent.press(screen.getByRole('radio', { name: 'Dark' }));

    expect(await screen.findByText('Could not save the color scheme.')).toBeOnTheScreen();
  });
});
