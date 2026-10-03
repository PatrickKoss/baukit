// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { announce } from '@baukit/a11y-core/web';
import type { ProductProfileErasureResult } from '@baukit/data-contracts';
import type { ProfileErasureOperation } from '@baukit/api-runtime/erasure';

import { deleteProfileCopy } from './delete-profile-copy';
import { DeleteProfileScreen } from './delete-profile-screen';

vi.mock('@baukit/a11y-core/web', () => ({ announce: vi.fn() }));

const receipt = { status: 'pending', operationId: 'erase-1' } as const;
const completed = { status: 'completed', operationId: 'erase-1' } as const;
const copy = deleteProfileCopy('en');
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

function confirm() {
  fireEvent.click(screen.getByRole('button', { name: copy.continue }));
  fireEvent.click(screen.getByRole('button', { name: copy.confirm }));
}

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('DeleteProfileScreen', () => {
  it('requires a second explicit step and focuses Cancel', () => {
    const erase = vi.fn(() => Promise.resolve(erased));
    render(<DeleteProfileScreen erase={erase} language="en" />);
    expect(screen.getByText(copy.description)).toBeDefined();
    expect(document.activeElement).toBe(screen.getByRole('heading', { name: copy.title }));
    fireEvent.click(screen.getByRole('button', { name: copy.continue }));
    expect(erase).not.toHaveBeenCalled();
    const cancel = screen.getByRole('button', { name: copy.cancel });
    expect(document.activeElement).toBe(cancel);
    fireEvent.click(cancel);
    expect(screen.queryByRole('button', { name: copy.confirm })).toBeNull();
    expect(erase).not.toHaveBeenCalled();
  });

  it('prevents a second request while erasure is running', async () => {
    let finish: (result: ProductProfileErasureResult) => void = () => {
      throw new Error('Request not started.');
    };
    const erase = vi.fn(
      () =>
        new Promise<ProductProfileErasureResult>((resolve) => {
          finish = resolve;
        }),
    );
    render(<DeleteProfileScreen erase={erase} language="en" />);
    confirm();
    expect(screen.getByRole('status').textContent).toBe(copy.busy);
    expect(screen.queryByRole('button', { name: copy.confirm })).toBeNull();
    expect(erase).toHaveBeenCalledOnce();
    await act(async () => {
      finish(erased);
      await Promise.resolve();
    });
    expect(screen.getByRole('status').textContent).toBe(copy.erased);
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
    'announces and focuses the %j outcome',
    async (result, message) => {
      render(<DeleteProfileScreen erase={() => Promise.resolve(result)} language="en" />);
      confirm();
      await screen.findByText(message);
      expect(document.activeElement).toBe(screen.getByRole('status'));
      expect(announce).toHaveBeenLastCalledWith(message);
      expect(screen.queryByRole('button', { name: copy.retry }) !== null).toBe(
        result.status === 'server-failure' || result.status === 'ambiguous',
      );
    },
  );

  it('retries an unknown outcome without another confirmation', async () => {
    const erase = vi
      .fn<() => Promise<ProductProfileErasureResult>>()
      .mockResolvedValueOnce({
        status: 'ambiguous',
        error: { stage: 'server', cause: 'Error' },
        warnings: [],
      })
      .mockResolvedValueOnce(pending);
    render(<DeleteProfileScreen erase={erase} language="en" />);
    confirm();
    fireEvent.click(await screen.findByRole('button', { name: copy.retry }));
    await screen.findByText(copy.pending);
    expect(erase).toHaveBeenCalledTimes(2);
  });

  it('shows unexpected rejections as unknown outcomes', async () => {
    render(<DeleteProfileScreen erase={() => Promise.reject(new Error('secret'))} language="en" />);
    confirm();
    await screen.findByText(copy.ambiguous);
    expect(screen.queryByText('secret')).toBeNull();
  });

  it.each(['completed', 'failed'] as const)(
    'shows a polled %s state without sending another deletion',
    async (status) => {
      const erase = vi.fn(() => Promise.resolve(pending));
      const poll = vi.fn(() => Promise.resolve({ status, operationId: 'erase-1' }));
      render(<DeleteProfileScreen erase={erase} poll={poll} language="en" />);
      confirm();
      fireEvent.click(await screen.findByRole('button', { name: copy.checkStatus }));
      await screen.findByText(status === 'completed' ? copy.erased : copy.failed);
      expect(poll).toHaveBeenCalledWith('erase-1', expect.any(AbortSignal));
      expect(erase).toHaveBeenCalledOnce();
      expect(screen.queryByRole('button', { name: copy.retry })).toBeNull();
    },
  );

  it('shows status errors while deletion continues and aborts polling on unmount', async () => {
    let signal: AbortSignal | undefined;
    const poll = vi
      .fn<(_id: string, signal: AbortSignal) => Promise<ProfileErasureOperation>>()
      .mockRejectedValueOnce(new Error('offline'))
      .mockImplementationOnce((_id, nextSignal) => {
        signal = nextSignal;
        return new Promise((_resolve, reject) => {
          nextSignal.addEventListener('abort', () => {
            reject(new Error('aborted'));
          });
        });
      });
    const { unmount } = render(
      <DeleteProfileScreen erase={() => Promise.resolve(pending)} poll={poll} language="en" />,
    );
    confirm();
    fireEvent.click(await screen.findByRole('button', { name: copy.checkStatus }));
    await screen.findByText(copy.statusError);
    fireEvent.click(screen.getByRole('button', { name: copy.checkStatus }));
    expect(signal?.aborted).toBe(false);
    unmount();
    expect(signal?.aborted).toBe(true);
  });

  it('renders German copy and disables deletion before identity is ready', () => {
    const german = deleteProfileCopy('de-DE');
    render(
      <DeleteProfileScreen
        erase={() => Promise.resolve(erased)}
        available={false}
        language="de-DE"
      />,
    );
    const button = screen.getByRole<HTMLButtonElement>('button', {
      name: german.continue,
    });
    expect(button.disabled).toBe(true);
    expect(screen.getByText(german.description)).toBeDefined();
  });
});
