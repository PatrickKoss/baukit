# @baukit/preferences-core

`@baukit/preferences-core` defines typed preferences and their update behavior. Products provide
the definitions, storage adapter, and any effects caused by a change.

## Update model

Each definition supplies a key, default value, normalizer, and scope. Hydration reads the store,
normalizes every stored field, and fills omitted fields from their definitions. A local update is
visible immediately, then written as a patch. If the write fails, the controller restores the
previous visible values and records the error.

Call `switchIdentity` with the next identity's store when the authenticated subject changes. The
controller publishes defaults before it starts the new read. A slow read or write for the old
identity cannot put that identity's preferences back into visible state, and a reported
side-effect failure that belongs to the replaced identity does not land on the new one.

Call `stop()` when whatever owns `onVisibleChange` goes away, such as a React provider
unmounting. In-flight work still settles and `getState()` stays accurate; the callback just stops
firing. Consumers do not need their own mounted flag around `onVisibleChange`.

Wire codecs distinguish three states by checking whether a payload owns a field:

- absent means an older peer did not send the setting, so no patch is made;
- null means the peer explicitly cleared a nullable setting;
- value contains the setting's decoded value.

An optional TypeScript property does not carry this distinction by itself.

## Side-effect ordering

The default policy is `after-persistence`. Locale changes, permission prompts, reminder
rescheduling, analytics changes, and network-query enablement should run only after the local
write succeeds. Every hook declares `onError: "report" | "ignore"`. A reported effect error stays
in controller state, while the persisted and visible preference remains unchanged.

Some visual changes need an immediate preview. A definition can opt into
`preview-with-rollback`, provide the preview operation, and provide its inverse. The controller
runs that rollback if preview or persistence fails. Its optional `afterPersistence` hook still
runs only after the write.

## Storage adapter tests

Adapters implement `PreferenceStore.read` and `PreferenceStore.patch`. Import
`describePreferenceStoreContract` from `@baukit/preferences-core/vitest` to run the shared Vitest
contract. `InMemoryPreferenceStore` is suitable for tests and small in-process consumers.

`RepositoryPreferenceStore` (or `createRepositoryPreferenceStore`) adapts a repository keyed by a
subject id, which is how most products already store a settings row. Give it the repository, the
subject id, and the two projections:

```ts
const store = createRepositoryPreferenceStore({
  repository: repositories.settings,
  subjectId: userId,
  toValues: (record) => ({ mode: record.theme, accent: record.accent_color }),
  toRecordPatch: (patch) => ({
    ...(patch.mode === undefined ? {} : { theme: patch.mode }),
    ...(patch.accent === undefined ? {} : { accent_color: patch.accent }),
  }),
});
```

`toRecordPatch` must omit keys the patch does not carry, so an untouched preference is never
written back. `read` resolves to `undefined` when the repository returns `null` or `undefined` for
that subject, which is what makes the controller fall back to definition defaults.

## Boundaries

This package does not generate SQL, choose a storage schema, run migrations, or render a settings
interface. Products still decide which preferences exist, where each scope is stored, what users
see, and how synchronized payloads reach a server.
