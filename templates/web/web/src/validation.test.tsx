// @vitest-environment jsdom

import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { focusFirstInvalid, validationAccessibilityProps } from './validation';
import { ValidationError } from './validation-error';

afterEach(cleanup);

describe('validation helpers', () => {
  it('links an invalid field to help and live error text', () => {
    expect(
      validationAccessibilityProps({
        describedBy: 'name-help',
        error: 'Enter a name.',
        errorId: 'name-error',
      }),
    ).toEqual({
      'aria-describedby': 'name-help name-error',
      'aria-invalid': true,
    });

    render(<ValidationError error="Enter a name." id="name-error" />);
    const alert = screen.getByRole('alert');
    expect(alert.id).toBe('name-error');
    expect(alert.getAttribute('aria-live')).toBe('assertive');
  });

  it('focuses the first invalid field', () => {
    const firstFocus = vi.fn();
    const secondFocus = vi.fn();

    expect(
      focusFirstInvalid([
        { invalid: false, element: { focus: firstFocus } },
        { invalid: true, element: { focus: secondFocus } },
        { invalid: true, element: { focus: firstFocus } },
      ]),
    ).toBe(true);
    expect(secondFocus).toHaveBeenCalledOnce();
    expect(firstFocus).not.toHaveBeenCalled();
  });
});
