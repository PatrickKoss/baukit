export function ValidationError({
  error,
  id,
}: {
  readonly error?: string | undefined;
  readonly id: string;
}) {
  if (error === undefined) {
    return null;
  }
  return (
    <p className="field-error" id={id} aria-live="assertive" role="alert">
      {error}
    </p>
  );
}
