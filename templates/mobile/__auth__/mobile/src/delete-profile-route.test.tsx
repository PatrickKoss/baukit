import { fireEvent, render, screen, waitFor } from '@testing-library/react-native';
import { Stack } from 'expo-router';

const mockErasePartition = jest.fn(() => Promise.resolve());

jest.mock('@baukit/a11y-core', () => ({
  announce: jest.fn(),
  focusAccessibilityElement: jest.fn(),
}));
jest.mock('expo-localization', () => ({
  getLocales: () => [{ languageTag: 'en' }],
}));
jest.mock('expo-router', () => ({
  ...jest.requireActual<typeof import('expo-router')>('expo-router'),
  Stack: { Screen: jest.fn(() => null) },
}));
jest.mock('./auth', () => ({
  useAuth: () => ({ subject: 'subject' }),
}));
jest.mock('./local-data', () => ({
  useAuthenticatedLocalData: () => ({
    state: { status: 'ready', partition: { subject: 'server-profile-id' } },
    erase: mockErasePartition,
  }),
}));
jest.mock('./app-shell', () => ({
  useAppPreferences: () => ({ resetPreferenceIdentity: () => Promise.resolve() }),
}));
jest.mock('./delete-profile', () => ({
  createDeleteProfileClient: jest.fn(),
  deleteProfile: jest.fn(),
}));

import DeleteProfileRoute from '../app/delete-profile';
import { englishErasureCopy, germanErasureCopy } from './delete-profile-copy';
import { initializeI18n } from './localization/i18n';
import { AppThemeProvider } from './theme';
import { createDeleteProfileClient, deleteProfile } from './delete-profile';

it.each([
  ['en', englishErasureCopy.title],
  ['de', germanErasureCopy.title],
])('uses the %s title in the native header and screen', async (language, title) => {
  await initializeI18n(language);
  await render(
    <AppThemeProvider mode="system" persistMode={() => Promise.resolve()}>
      <DeleteProfileRoute />
    </AppThemeProvider>,
  );
  expect(Stack.Screen).toHaveBeenLastCalledWith({ options: { title } }, undefined);
  expect(screen.getByText(title)).toBeOnTheScreen();
});

it('erases the server profile partition while authorizing deletion with the auth subject', async () => {
  await initializeI18n('en');
  jest.mocked(deleteProfile).mockImplementation(async (options) => {
    await options.eraseLocalPartition();
    return { status: 'erased', receipt: { status: 'completed', operationId: 'erasure' }, warnings: [] };
  });
  await render(
    <AppThemeProvider mode="system" persistMode={() => Promise.resolve()}>
      <DeleteProfileRoute />
    </AppThemeProvider>,
  );
  await fireEvent.press(screen.getByRole('button', { name: englishErasureCopy.continue }));
  await fireEvent.press(screen.getByRole('button', { name: englishErasureCopy.confirm }));
  await waitFor(() => { expect(mockErasePartition).toHaveBeenCalledWith('server-profile-id'); });
  expect(createDeleteProfileClient).toHaveBeenCalledWith('subject');
  expect(deleteProfile).toHaveBeenCalledWith(expect.objectContaining({ subject: 'subject' }));
});
