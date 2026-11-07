# TypeScript fixes

## `@typescript-eslint/strict-boolean-expressions`

```ts
// before
if (user.name) render(user.name);
if (items.length) flush(items);
// after
if (user.name !== undefined && user.name !== '') render(user.name);
if (items.length > 0) flush(items);
```

## `max-params` (4)

```ts
// before
export function schedule(id: string, at: Date, tz: string, repeat: boolean, label: string): Job {
// after
export interface ScheduleRequest {
  id: string;
  at: Date;
  timeZone: string;
  repeat: boolean;
  label: string;
}
export function schedule(request: ScheduleRequest): Job {
```

## `max-depth` (4) and `complexity` (15)

Return early and extract the inner loop body.

```ts
// before
for (const row of rows) {
  if (row.active) {
    if (row.owner === owner) {
      if (row.items.length > 0) {
        for (const item of row.items) { /* ... */ }
      }
    }
  }
}
// after
for (const row of rows.filter((candidate) => isOwnedActive(candidate, owner))) {
  applyItems(row.items);
}
```

## `consistent-type-assertions` and `ts-double-cast`

```ts
// before
const body = (await response.json()) as unknown as Session;
// after
const body: unknown = await response.json();
if (!isSession(body)) throw new ApiContractError('session');
```

`isSession` is built from `@baukit/type-guards` combinators. Object literal assertions such as
`{} as Settings` become `const settings: Settings = { ... }` or `{ ... } satisfies Settings`.

## `explicit-module-boundary-types`

```ts
// before
export function formatAmount(cents: number, currency: string) {
// after
export function formatAmount(cents: number, currency: string): string {
```
