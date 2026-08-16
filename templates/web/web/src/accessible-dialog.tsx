import { useId, useRef, useState, type ReactNode, type RefObject } from 'react';
import { createPortal } from 'react-dom';

import { useFocusTrap } from './use-focus-trap';
import { useInert } from './use-inert';
import { useSingleFlight } from './use-single-flight';
import { focusFirstInvalid, validationAccessibilityProps } from './validation';
import { ValidationError } from './validation-error';

interface AccessibleDialogProps {
  readonly children: ReactNode;
  readonly initialFocusRef?: RefObject<HTMLElement | null> | undefined;
  readonly onClose: () => void;
  readonly open: boolean;
  readonly title: string;
}

export function AccessibleDialog({
  children,
  initialFocusRef,
  onClose,
  open,
  title,
}: AccessibleDialogProps) {
  const backdropRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const titleId = useId();

  useInert(backdropRef, open);
  useFocusTrap({
    active: open,
    containerRef: dialogRef,
    initialFocusRef,
    onEscape: onClose,
  });

  if (!open) {
    return null;
  }

  return createPortal(
    <div className="dialog-backdrop" ref={backdropRef}>
      <section
        className="dialog"
        ref={dialogRef}
        aria-labelledby={titleId}
        aria-modal="true"
        role="dialog"
        tabIndex={-1}
      >
        <h2 id={titleId}>{title}</h2>
        {children}
      </section>
    </div>,
    document.body,
  );
}

export function AccessibleDialogExample() {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [error, setError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const runMutation = useSingleFlight();

  function close(): void {
    setOpen(false);
    setError(undefined);
    setSaved(false);
  }

  function submit(event: { preventDefault: () => void }): void {
    event.preventDefault();
    void runMutation(() => {
      if (name.trim().length === 0) {
        setError('Enter a name for this example.');
        focusFirstInvalid([{ invalid: true, element: inputRef.current }]);
        return Promise.resolve();
      }
      setError(undefined);
      setSaved(true);
      return Promise.resolve();
    });
  }

  return (
    <section className="panel" aria-labelledby="interaction-title">
      <h2 id="interaction-title">Interaction reference</h2>
      <p className="muted">
        This small example demonstrates modal focus, background inertness, linked validation, and a
        same-tick mutation guard.
      </p>
      <button
        className="action"
        type="button"
        onClick={() => {
          setOpen(true);
        }}
      >
        Open dialog example
      </button>
      <AccessibleDialog
        initialFocusRef={inputRef}
        onClose={close}
        open={open}
        title="Accessible dialog example"
      >
        <form onSubmit={submit} noValidate>
          <label htmlFor="dialog-name">Example name</label>
          <input
            id="dialog-name"
            ref={inputRef}
            value={name}
            onChange={(event) => {
              setName(event.currentTarget.value);
            }}
            {...validationAccessibilityProps({
              describedBy: 'dialog-name-help',
              error,
              errorId: 'dialog-name-error',
            })}
          />
          <p className="field-help" id="dialog-name-help">
            Validation keeps its help and error linked to this field.
          </p>
          <ValidationError error={error} id="dialog-name-error" />
          {saved ? (
            <p className="success" aria-live="polite" role="status">
              Saved once.
            </p>
          ) : null}
          <div className="actions dialog-actions">
            <button className="action secondary" type="button" onClick={close}>
              Cancel
            </button>
            <button className="action" type="submit">
              Save example
            </button>
          </div>
        </form>
      </AccessibleDialog>
    </section>
  );
}
