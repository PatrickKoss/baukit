import { act, fireEvent, render, screen } from '@testing-library/react-native';
import { announce, focusAccessibilityElement } from '@baukit/a11y-core';
import type { ProductProfileErasureResult } from '@baukit/data-contracts';
import type { ProfileErasureOperation } from '@baukit/api-runtime/erasure';

jest.mock('@baukit/a11y-core', () => ({
  announce: jest.fn(),
  focusAccessibilityElement: jest.fn(),
}));
jest.mock('expo-localization', () => ({
  getLocales: () => [{ languageTag: 'en' }],
}));

import { englishErasureCopy as copy, germanErasureCopy } from './delete-profile-copy';
import { DeleteProfileScreen } from './delete-profile-screen';
import { initializeI18n } from './localization/i18n';
import { AppThemeProvider } from './theme';

const receipt = { status: 'pending', operationId: 'erase-1' } as const;
const completed = { status: 'completed', operationId: 'erase-1' } as const;
const erased: ProductProfileErasureResult = {
  status: 'erased',
  receipt: completed,
  warnings: [],
};
const pending: ProductProfileErasureResult = {
  status: 'pending',
  receipt,
  warnings: [],
};

function renderScreen(
  erase: () => Promise<ProductProfileErasureResult>,
  poll?: (id: string, signal: AbortSignal) => Promise<ProfileErasureOperation>,
  available = true,
) {
  return render(
    <AppThemeProvider mode="system" persistMode={() => Promise.resolve()}>
      <DeleteProfileScreen
        erase={erase}
        {...(poll === undefined ? {} : { poll })}
        available={available}
      />
    </AppThemeProvider>,
  );
}

async function confirm() {
  await fireEvent.press(screen.getByRole('button', { name: copy.continue }));
  await fireEvent.press(screen.getByRole('button', { name: copy.confirm }));
}

beforeEach(async () => {
  await initializeI18n('en');
});

describe('DeleteProfileScreen', () => {
  it('requires a second explicit step and allows cancellation', async () => {
    const erase = jest.fn(() => Promise.resolve(erased));
    await renderScreen(erase);
    expect(screen.getByText(copy.description)).toBeOnTheScreen();
    expect(focusAccessibilityElement).toHaveBeenCalled();
    await fireEvent.press(screen.getByRole('button', { name: copy.continue }));
    expect(erase).not.toHaveBeenCalled();
    expect(announce).toHaveBeenLastCalledWith(copy.confirmation);
    await fireEvent.press(screen.getByRole('button', { name: copy.cancel }));
    expect(screen.queryByRole('button', { name: copy.confirm })).toBeNull();
    expect(erase).not.toHaveBeenCalled();
  });

  it('prevents a second request while erasure is running', async () => {
    let finish: (result: ProductProfileErasureResult) => void = () => {
      throw new Error('Request not started.');
    };
    const erase = jest.fn(
      () =>
        new Promise<ProductProfileErasureResult>((resolve) => {
          finish = resolve;
        }),
    );
    await renderScreen(erase);
    await confirm();
    expect(screen.getByText(copy.busy)).toBeOnTheScreen();
    expect(screen.queryByRole('button', { name: copy.confirm })).toBeNull();
    expect(erase).toHaveBeenCalledTimes(1);
    await act(() => {
      finish(erased);
    });
    expect(screen.getByText(copy.erased)).toBeOnTheScreen();
  });

  it.each([
    [erased, copy.erased],
    [pending, copy.pending],
    [
      {
        status: 'server-failure',
        error: { stage: 'server', cause: 'Error' },
        warnings: [],
      },
      copy.serverFailure,
    ],
    [
      {
        status: 'ambiguous',
        error: { stage: 'server', cause: 'TypeError' },
        warnings: [],
      },
      copy.ambiguous,
    ],
    [
      {
        status: 'local-failure',
        receipt,
        error: { stage: 'local', cause: 'Error' },
        signOutError: null,
        warnings: [],
      },
      copy.localFailure,
    ],
    [
      {
        status: 'signout-failure',
        receipt,
        error: { stage: 'sign-out', cause: 'Error' },
        warnings: [],
      },
      copy.signoutFailure,
    ],
  ] satisfies readonly (readonly [ProductProfileErasureResult, string])[])(
    'announces the %j outcome',
    async (result, message) => {
      await renderScreen(() => Promise.resolve(result));
      await confirm();
      expect(await screen.findByText(message)).toBeOnTheScreen();
      expect(announce).toHaveBeenLastCalledWith(message);
      expect(screen.queryByRole('button', { name: copy.retry }) !== null).toBe(
        result.status === 'server-failure' || result.status === 'ambiguous',
      );
    },
  );

  it('retries an unknown outcome without another confirmation', async () => {
    const erase = jest
      .fn<Promise<ProductProfileErasureResult>, []>()
      .mockResolvedValueOnce({
        status: 'ambiguous',
        error: { stage: 'server', cause: 'Error' },
        warnings: [],
      })
      .mockResolvedValueOnce(pending);
    await renderScreen(erase);
    await confirm();
    await fireEvent.press(await screen.findByRole('button', { name: copy.retry }));
    expect(await screen.findByText(copy.pending)).toBeOnTheScreen();
    expect(erase).toHaveBeenCalledTimes(2);
  });

  it('shows unexpected rejections as unknown outcomes', async () => {
    await renderScreen(() => Promise.reject(new Error('secret')));
    await confirm();
    expect(await screen.findByText(copy.ambiguous)).toBeOnTheScreen();
    expect(screen.queryByText('secret')).toBeNull();
  });

  it.each(['completed', 'failed'] as const)(
    'shows a polled %s state without another deletion',
    async (status) => {
      const erase = jest.fn(() => Promise.resolve(pending));
      const poll = jest.fn(() => Promise.resolve({ status, operationId: 'erase-1' }));
      await renderScreen(erase, poll);
      await confirm();
      await fireEvent.press(await screen.findByRole('button', { name: copy.checkStatus }));
      expect(
        await screen.findByText(status === 'completed' ? copy.erased : copy.failed),
      ).toBeOnTheScreen();
      expect(poll).toHaveBeenCalledWith('erase-1', expect.any(AbortSignal));
      expect(erase).toHaveBeenCalledTimes(1);
      expect(screen.queryByRole('button', { name: copy.retry })).toBeNull();
    },
  );

  it('shows status errors and aborts a new poll on unmount', async () => {
    let signal: AbortSignal | undefined;
    const poll = jest
      .fn<Promise<ProfileErasureOperation>, [string, AbortSignal]>()
      .mockRejectedValueOnce(new Error('offline'))
      .mockImplementationOnce((_id, nextSignal) => {
        signal = nextSignal;
        return new Promise((_resolve, reject) => {
          nextSignal.addEventListener('abort', () => {
            reject(new Error('aborted'));
          });
        });
      });
    const { unmount } = await renderScreen(() => Promise.resolve(pending), poll);
    await confirm();
    await fireEvent.press(await screen.findByRole('button', { name: copy.checkStatus }));
    expect(await screen.findByText(copy.statusError)).toBeOnTheScreen();
    await fireEvent.press(screen.getByRole('button', { name: copy.checkStatus }));
    expect(signal?.aborted).toBe(false);
    await unmount();
    expect(signal?.aborted).toBe(true);
  });

  it('renders German strings through i18next and disables deletion before identity is ready', async () => {
    await initializeI18n('de');
    await renderScreen(() => Promise.resolve(erased), undefined, false);
    expect(screen.getByRole('button', { name: germanErasureCopy.continue })).toBeDisabled();
    expect(screen.getByText(germanErasureCopy.description)).toBeOnTheScreen();
  });
});
