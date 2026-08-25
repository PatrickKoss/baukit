// @vitest-environment jsdom

import { act, cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { AccessibleDialogExample } from './accessible-dialog';

afterEach(cleanup);

/** The trap restores focus on the next task so it never lands on a still-inert trigger. */
async function flushFocusRestore(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe('AccessibleDialogExample', () => {
  it('contains focus, makes the background inert, closes on Escape, and restores focus', async () => {
    const result = render(<AccessibleDialogExample />);
    const opener = screen.getByRole('button', { name: 'Open dialog example' });
    opener.focus();
    fireEvent.click(opener);

    const dialog = screen.getByRole('dialog', {
      name: 'Accessible dialog example',
    });
    const input = screen.getByRole('textbox', { name: 'Example name' });
    const save = screen.getByRole('button', { name: 'Save example' });
    expect(document.activeElement).toBe(input);
    expect(result.container.hasAttribute('inert')).toBe(true);

    save.focus();
    fireEvent.keyDown(save, { key: 'Tab' });
    expect(document.activeElement).toBe(input);

    input.focus();
    fireEvent.keyDown(input, { key: 'Tab', shiftKey: true });
    expect(document.activeElement).toBe(save);

    fireEvent.keyDown(save, { key: 'Escape' });
    expect(dialog.isConnected).toBe(false);
    expect(result.container.hasAttribute('inert')).toBe(false);

    await flushFocusRestore();
    expect(document.activeElement).toBe(opener);
  });

  it('announces validation and focuses the first invalid field', () => {
    render(<AccessibleDialogExample />);
    fireEvent.click(screen.getByRole('button', { name: 'Open dialog example' }));
    fireEvent.click(screen.getByRole('button', { name: 'Save example' }));

    const input = screen.getByRole('textbox', { name: 'Example name' });
    const alert = screen.getByRole('alert');
    expect(input.getAttribute('aria-invalid')).toBe('true');
    expect(input.getAttribute('aria-describedby')).toBe('dialog-name-help dialog-name-error');
    expect(alert.textContent).toBe('Enter a name for this example.');
    expect(document.activeElement).toBe(input);
  });
});
