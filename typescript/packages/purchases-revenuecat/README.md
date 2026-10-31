# `@baukit/purchases-revenuecat`

The `react-native-purchases` implementation of the `@baukit/purchases-core` port. It imports only types from `react-native-purchases`. The app passes the module in.

```ts
import Purchases from 'react-native-purchases';
import { createRevenueCatPurchasesPort } from '@baukit/purchases-revenuecat';

const port = createRevenueCatPurchasesPort(Purchases, { apiKey: platformApiKey });
```

- The factory calls `Purchases.configure({ apiKey })` once and throws `unavailable` for an empty key. Use `createNoopPurchasesPort()` when the build has no key.
- `identify` calls `logIn` with the suite subject `<identity_domain>|<hub sub>`. The hub resolves RevenueCat webhooks against that string.
- `reset` calls `logOut` unless the user is already anonymous.
- `offerings` maps RevenueCat offerings. Offering and product IDs come from the RevenueCat dashboard and app configuration. `NON_SUBSCRIPTION` products become `one_time` packages for credit packs.
- RevenueCat error codes map to `cancelled`, `pending`, `network` or `store`.

`react-native-purchases` 10.14.0 or later is a peer dependency. It needs no Expo config plugin. Expo Go has no store access, so use a development build.
