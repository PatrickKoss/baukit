import { useId, useRef, useState, type ReactElement, type ReactNode, type RefObject } from 'react';
import { createPortal } from 'react-dom';

import { useFocusTrap, useInert, useSingleFlight } from '@baukit/a11y-core/web';

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
}: AccessibleDialogProps): ReactElement | null {
  const backdropRef = useRef<HTMLDivElement>(null);
  const dialogRef = useRef<HTMLElement>(null);
  const titleId = useId();

  useInert(backdropRef, open);
  const { onKeyDown } = useFocusTrap({
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
        onKeyDown={onKeyDown}
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

interface ValidatedFieldProps {
  readonly id: string;
  readonly label: string;
  readonly help: string;
  readonly inputRef: RefObject<HTMLInputElement | null>;
  readonly value: string;
  readonly onChange: (value: string) => void;
  readonly error: string | undefined;
}

function ValidatedField({
  id,
  label,
  help,
  inputRef,
  value,
  onChange,
  error,
}: ValidatedFieldProps): ReactElement {
  return (
    <>
      <label htmlFor={id}>{label}</label>
      <input
        id={id}
        ref={inputRef}
        value={value}
        onChange={(event) => {
          onChange(event.currentTarget.value);
        }}
        {...validationAccessibilityProps({
          describedBy: `${id}-help`,
          error,
          errorId: `${id}-error`,
        })}
      />
      <p className="field-help" id={`${id}-help`}>
        {help}
      </p>
      <ValidationError error={error} id={`${id}-error`} />
    </>
  );
}

function useExampleForm() {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState('');
  const [note, setNote] = useState('');
  const [nameError, setNameError] = useState<string>();
  const [noteError, setNoteError] = useState<string>();
  const [saved, setSaved] = useState(false);
  const nameRef = useRef<HTMLInputElement>(null);
  const noteRef = useRef<HTMLInputElement>(null);
  const runMutation = useSingleFlight();

  function close(): void {
    setOpen(false);
    setNameError(undefined);
    setNoteError(undefined);
    setSaved(false);
  }

  function submit(event: { preventDefault: () => void }): void {
    event.preventDefault();
    void runMutation(() => {
      const nextNameError = name.trim().length === 0 ? 'Enter a name for this example.' : undefined;
      const nextNoteError = note.trim().length === 0 ? 'Enter a note for this example.' : undefined;
      setNameError(nextNameError);
      setNoteError(nextNoteError);
      if (nextNameError !== undefined || nextNoteError !== undefined) {
        focusFirstInvalid([
          { invalid: nextNameError !== undefined, element: nameRef.current },
          { invalid: nextNoteError !== undefined, element: noteRef.current },
        ]);
        return Promise.resolve();
      }
      setSaved(true);
      return Promise.resolve();
    });
  }

  return {
    open,
    show: (): void => {
      setOpen(true);
    },
    close,
    submit,
    saved,
    name: { value: name, onChange: setName, error: nameError, inputRef: nameRef },
    note: { value: note, onChange: setNote, error: noteError, inputRef: noteRef },
  };
}

export function AccessibleDialogExample(): ReactElement {
  const form = useExampleForm();
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
        onClick={form.show}
      >
        Open dialog example
      </button>
      <AccessibleDialog
        initialFocusRef={form.name.inputRef}
        onClose={form.close}
        open={form.open}
        title="Accessible dialog example"
      >
        <form onSubmit={form.submit} noValidate>
          <ValidatedField
            id="dialog-name"
            label="Example name"
            help="Validation keeps its help and error linked to this field."
            {...form.name}
          />
          <ValidatedField
            id="dialog-note"
            label="Example note"
            help="Submit guards must retain every entered field after another field fails validation."
            {...form.note}
          />
          {form.saved ? (
            <p className="success" aria-live="polite" role="status">
              Saved once.
            </p>
          ) : null}
          <div className="actions dialog-actions">
            <button className="action secondary" type="button" onClick={form.close}>
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
