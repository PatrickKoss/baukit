import type { AriaAttributes } from 'react';

export interface InvalidFieldTarget {
  readonly invalid: boolean;
  readonly element: Pick<HTMLElement, 'focus'> | null;
}

export function focusFirstInvalid(fields: readonly InvalidFieldTarget[]): boolean {
  const first = fields.find(({ element, invalid }) => invalid && element !== null);
  if (first?.element === null || first?.element === undefined) {
    return false;
  }
  first.element.focus();
  return true;
}

export function validationAccessibilityProps({
  describedBy,
  error,
  errorId,
}: {
  readonly describedBy?: string | undefined;
  readonly error?: string | undefined;
  readonly errorId: string;
}): Pick<AriaAttributes, 'aria-describedby' | 'aria-invalid'> {
  const descriptionIds = [describedBy, error === undefined ? undefined : errorId].filter(
    (id): id is string => id !== undefined && id.length > 0,
  );
  return {
    'aria-describedby': descriptionIds.length === 0 ? undefined : descriptionIds.join(' '),
    'aria-invalid': error === undefined ? undefined : true,
  };
}
