import { render, screen } from '@testing-library/react-native';
import { Stack } from 'expo-router';

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
  useOidcAuth: () => ({ subject: 'subject' }),
}));
jest.mock('./local-data', () => ({
  useAuthenticatedLocalData: () => ({
    state: { status: 'ready' },
    erase: () => Promise.resolve(),
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
